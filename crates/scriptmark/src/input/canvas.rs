//! The Canvas entry point: turn Canvas API payloads into the unified input.
//!
//! [`normalize`] is pure — it performs no I/O. Fetching, pagination and attachment
//! download are P-670; this module defines the payload shapes that work lands in, so the
//! HTTP layer can be written and tested against exactly what normalisation consumes.
//!
//! The payload types live here rather than in [`crate::canvas::client`] on purpose: that
//! module's `CanvasSubmission` is the *grade-push response* shape, and its types are not
//! re-exported.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::discovery::detect_language;
use crate::models::{
	Assignment, AssignmentInput, Attachment, AttemptPolicy, DiagnosticKind, FileOrigin,
	InputDiagnostic, InputSource, RosterMatch, SourceStatus, StudentFile, StudentIdentity,
	StudentKey, StudentSubmission, SubmissionAttempt, is_reserved_key, normalize_key,
};
use crate::roster::{Roster, RosterEntry, RosterSource};

/// A course user, as `GET /courses/:id/users` returns it.
///
/// `sis_user_id` is `Option<String>` and nothing else: a payload sending it unquoted is a
/// deserialisation error rather than a silent number→string coercion, which is what keeps
/// a 学号 like `0024010003` from arriving as `24010003`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasUserPayload {
	pub id: u64,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub sortable_name: Option<String>,
	#[serde(default)]
	pub sis_user_id: Option<String>,
	#[serde(default)]
	pub login_id: Option<String>,
	#[serde(default)]
	pub email: Option<String>,
}

/// A course, as `GET /courses` returns it. Used only by `canvas courses`, which exists so a
/// teacher can find an id without leaving the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasCoursePayload {
	pub id: u64,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub course_code: Option<String>,
	#[serde(default)]
	pub term: Option<CanvasTermPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasTermPayload {
	#[serde(default)]
	pub name: Option<String>,
}

/// An assignment, as `GET /courses/:c/assignments/:a` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasAssignmentPayload {
	pub id: u64,
	#[serde(default)]
	pub name: Option<String>,
	/// Shown in `canvas assignments` so a teacher can tell two similarly named ones apart.
	#[serde(default)]
	pub due_at: Option<String>,
}

/// An uploaded file, as Canvas reports it on a submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasAttachmentPayload {
	pub id: u64,
	#[serde(default)]
	pub filename: Option<String>,
	#[serde(default)]
	pub display_name: Option<String>,
	/// Canvas spells this with a hyphen: its attachment serialiser emits
	/// `"content-type" => attachment.content_type`. Without the rename this field never
	/// populated from a real payload — only from our own underscore-spelled fixtures,
	/// which is why the gap survived P-669. `rename` makes the wire spelling the one we
	/// write back out, so a saved bundle round-trips; `alias` keeps those fixtures loading.
	#[serde(default, rename = "content-type", alias = "content_type")]
	pub content_type: Option<String>,
	#[serde(default)]
	pub size: Option<u64>,
	#[serde(default)]
	pub url: Option<String>,
}

impl CanvasAttachmentPayload {
	/// The name Canvas reports. `pub(crate)` because the bundle writer and the offline
	/// loader both have to derive the same on-disk path from it — see `bundle::disk_name`.
	pub(crate) fn name(&self) -> String {
		self.display_name
			.clone()
			.or_else(|| self.filename.clone())
			.unwrap_or_else(|| format!("attachment-{}", self.id))
	}
}

/// A submission, as `GET /courses/:id/assignments/:id/submissions` returns it.
///
/// Canvas emits a placeholder row for every enrolled student, so `attempt: None` with
/// `workflow_state: "unsubmitted"` means *nothing was handed in* — not an empty hand-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasSubmissionPayload {
	#[serde(default)]
	pub id: u64,
	pub user_id: u64,
	#[serde(default)]
	pub attempt: Option<u32>,
	#[serde(default)]
	pub workflow_state: Option<String>,
	#[serde(default)]
	pub submitted_at: Option<String>,
	#[serde(default)]
	pub late: bool,
	#[serde(default)]
	pub missing: bool,
	#[serde(default)]
	pub excused: Option<bool>,
	#[serde(default)]
	pub submission_type: Option<String>,
	#[serde(default)]
	pub body: Option<String>,
	#[serde(default)]
	pub attachments: Vec<CanvasAttachmentPayload>,
	/// Earlier attempts, when the request asked for them.
	#[serde(default)]
	pub submission_history: Vec<CanvasSubmissionPayload>,
}

impl CanvasSubmissionPayload {
	fn source_status(&self) -> SourceStatus {
		SourceStatus {
			workflow_state: self
				.workflow_state
				.clone()
				.unwrap_or_else(|| "unknown".to_string()),
			late: self.late,
			missing: self.missing,
			excused: self.excused.unwrap_or(false),
		}
	}

	fn has_text_body(&self) -> bool {
		self.body.as_deref().is_some_and(|b| !b.trim().is_empty())
	}
}

/// Everything one Canvas import fetched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasPayload {
	#[serde(default)]
	pub course_id: Option<u64>,
	#[serde(default)]
	pub assignment_id: Option<u64>,
	#[serde(default)]
	pub assignment_name: Option<String>,
	#[serde(default)]
	pub users: Vec<CanvasUserPayload>,
	#[serde(default)]
	pub submissions: Vec<CanvasSubmissionPayload>,
}

/// Where each downloaded attachment landed, keyed by attachment id.
///
/// P-669 does no downloading, so this is supplied by the caller: the fixtures point it at
/// committed files, and P-670 fills it after fetching. An attachment that is missing from
/// it is reported as [`DiagnosticKind::PendingDownload`] and never makes a student
/// executable.
pub type DownloadedAttachments = HashMap<u64, PathBuf>;

/// Turn Canvas payloads into the unified input.
///
/// Canvas decides who is in the course. The roster is the union of course enrollment and
/// whatever the teacher supplied: enrollment settles membership, so somebody Canvas lists
/// is never called a stranger because a spreadsheet is out of date, and a non-submitter
/// still appears rather than vanishing. A supplied row Canvas has never heard of is kept
/// and flagged `NotEnrolled`, so a hand-maintained list cannot lose people either.
pub fn normalize(
	payload: &CanvasPayload,
	roster: Option<&Roster>,
	downloads: &DownloadedAttachments,
	policy: AttemptPolicy,
) -> AssignmentInput {
	let mut diagnostics: Vec<InputDiagnostic> = Vec::new();

	let users: BTreeMap<u64, &CanvasUserPayload> =
		payload.users.iter().map(|u| (u.id, u)).collect();

	let roster = merged_roster(&payload.users, roster, &mut diagnostics);

	let mut students: Vec<StudentSubmission> = Vec::new();
	// Keyed on the value, not its rendering — `Display` prefixes are not escaped.
	let mut covered: std::collections::BTreeSet<StudentKey> = Default::default();
	let mut covered_canvas_ids: std::collections::BTreeSet<u64> = Default::default();
	let mut seen_users: std::collections::BTreeSet<u64> = Default::default();

	// Deterministic regardless of payload order.
	let mut submissions: Vec<&CanvasSubmissionPayload> = payload.submissions.iter().collect();
	submissions.sort_by_key(|s| s.user_id);

	for submission in submissions {
		if !seen_users.insert(submission.user_id) {
			diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::DuplicateSubmissionRow {
					canvas_user_id: submission.user_id,
				},
			));
			continue;
		}
		let user = users.get(&submission.user_id).copied();
		let mut identity = identity_for(submission.user_id, user, &mut diagnostics);

		let attempts = attempts_of(submission, downloads, &mut diagnostics, &identity);

		// A placeholder row for someone the roster covers is handled by the roster merge
		// below, which is the only producer of NotSubmitted. A placeholder row for someone
		// the roster does not cover describes a non-event: nothing arrived and nobody is
		// owed a grade.
		if attempts.is_empty() {
			continue;
		}

		if submission.attachments.is_empty() && submission.has_text_body() {
			diagnostics.push(InputDiagnostic::warning(DiagnosticKind::TextEntryOnly {
				key: identity.key.raw(),
			}));
		}

		let roster_match = match roster.lookup(&identity.key) {
			Some(i) => {
				covered.insert(identity.key.clone());
				covered_canvas_ids.extend(identity.canvas_user_id);
				if identity.name.is_none() {
					identity.name = roster.entries[i].name.clone();
				}
				RosterMatch::Matched(i)
			}
			None => {
				diagnostics.push(InputDiagnostic::warning(DiagnosticKind::NotOnRoster {
					key: identity.key.raw(),
				}));
				RosterMatch::NotInRoster
			}
		};

		students.push(StudentSubmission::received(
			identity,
			roster_match,
			attempts,
			policy,
		));
	}

	// Canvas knows who these people are even though the teacher's CSV only carries a
	// number, so their Canvas identity is filled in from enrollment rather than lost.
	let mut by_number: BTreeMap<String, Vec<&CanvasUserPayload>> = BTreeMap::new();
	for user in &payload.users {
		if let Some(number) = user.sis_user_id.as_deref().map(normalize_key)
			&& !number.is_empty()
			&& !is_reserved_key(&number)
		{
			by_number.entry(number).or_default().push(user);
		}
	}

	for entry in &roster.entries {
		if !covered.insert(entry.key.clone()) {
			continue;
		}
		// A roster that names one person under both a 学号 and a Canvas id must not count
		// them twice.
		if entry
			.canvas_user_id
			.is_some_and(|id| covered_canvas_ids.contains(&id))
		{
			continue;
		}
		covered_canvas_ids.extend(entry.canvas_user_id);

		let Some(index) = roster.lookup(&entry.key) else {
			continue;
		};
		let mut identity = match &entry.key {
			StudentKey::CanvasUser(id) => StudentIdentity::canvas_user(*id),
			key => StudentIdentity::number(key.raw()),
		};
		identity.name = entry.name.clone();
		identity.canvas_user_id = identity.canvas_user_id.or(entry.canvas_user_id);

		// Enrich from enrollment. A Canvas-keyed row resolves by Canvas id; a 学号 row
		// resolves by student number — never through `raw()`, which would compare a Canvas
		// id against other people's SIS ids and walk straight through the namespace
		// boundary `lookup` exists to hold.
		let enrolled = match &entry.key {
			StudentKey::CanvasUser(id) => users.get(id).copied(),
			_ => entry
				.student_number()
				.and_then(|number| by_number.get(number))
				.filter(|candidates| candidates.len() == 1)
				.map(|candidates| candidates[0]),
		};
		if let Some(user) = enrolled {
			identity.canvas_user_id = identity.canvas_user_id.or(Some(user.id));
			identity.sis_user_id = user.sis_user_id.as_deref().map(normalize_key);
			identity.login_id = user.login_id.clone();
			identity.sortable_name = user.sortable_name.clone();
			identity.email = user.email.clone();
			if identity.name.is_none() {
				identity.name = user.name.clone();
			}
		}
		students.push(StudentSubmission::not_submitted(identity, index));
	}

	diagnostics.extend(roster.diagnostics.iter().cloned());
	diagnostics.dedup();

	let mut input = AssignmentInput {
		assignment: Assignment {
			name: payload.assignment_name.clone().unwrap_or_default(),
			canvas_course_id: payload.course_id,
			canvas_assignment_id: payload.assignment_id,
			..Assignment::default()
		},
		source: InputSource::Canvas {
			course_id: payload.course_id,
			assignment_id: payload.assignment_id,
		},
		roster: Some(roster),
		students,
		unmatched: Vec::new(),
		diagnostics,
	};

	let zero_padded = input.detect_zero_padded_variants();
	input.diagnostics.extend(zero_padded);

	input.sorted()
}

/// The roster of record on the Canvas path: course enrollment, plus any supplied row it
/// does not already cover.
///
/// Canvas is authoritative about who is in the course — an enrollee carrying no SIS id is
/// keyed by their Canvas id rather than dropped, and a submitter Canvas knows about is
/// never called a stranger because a teacher's spreadsheet is out of date. A supplied row
/// for somebody Canvas has never heard of is still kept, marked as supplied and flagged,
/// so a hand-maintained list cannot silently lose people either.
fn merged_roster(
	users: &[CanvasUserPayload],
	supplied: Option<&Roster>,
	diagnostics: &mut Vec<InputDiagnostic>,
) -> Roster {
	let mut entries: Vec<RosterEntry> = users
		.iter()
		.map(|u| {
			let number = u
				.sis_user_id
				.as_deref()
				.map(normalize_key)
				.filter(|n| !n.is_empty() && !is_reserved_key(n));
			RosterEntry {
				key: match number {
					Some(number) => StudentKey::Number(number),
					None => StudentKey::CanvasUser(u.id),
				},
				source: RosterSource::CanvasEnrollment,
				name: u.name.clone(),
				canvas_user_id: Some(u.id),
				location: None,
			}
		})
		.collect();
	entries.sort_by(|a, b| (&a.key, a.canvas_user_id).cmp(&(&b.key, b.canvas_user_id)));

	let mut carried = Vec::new();
	if let Some(supplied) = supplied {
		let enrolled: std::collections::BTreeSet<&StudentKey> =
			entries.iter().map(|e| &e.key).collect();
		for entry in &supplied.entries {
			if enrolled.contains(&entry.key) {
				continue;
			}
			diagnostics.push(InputDiagnostic::warning(DiagnosticKind::NotEnrolled {
				key: entry.key.raw(),
			}));
			carried.push(entry.clone());
		}
		diagnostics.extend(supplied.diagnostics.iter().cloned());
	}
	entries.extend(carried);

	Roster::from_entries(entries)
}

fn identity_for(
	user_id: u64,
	user: Option<&CanvasUserPayload>,
	diagnostics: &mut Vec<InputDiagnostic>,
) -> StudentIdentity {
	let sis = user
		.and_then(|u| u.sis_user_id.as_deref())
		.map(normalize_key)
		.filter(|s| !s.is_empty())
		// The same rule the roster loader applies: a 学号 beginning with a reserved
		// prefix would render as a key of another kind and stop round-tripping.
		.filter(|s| !is_reserved_key(s));

	let mut identity = match &sis {
		// Canvas vouches for its own SIS id, so this is a confirmed 学号 even before a
		// roster is consulted.
		Some(number) => StudentIdentity::number(number),
		None => {
			diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::MissingStudentNumber {
					canvas_user_id: user_id,
				},
			));
			StudentIdentity::canvas_user(user_id)
		}
	};

	identity.canvas_user_id = Some(user_id);
	identity.sis_user_id = sis;
	if let Some(user) = user {
		identity.name = user.name.clone();
		identity.sortable_name = user.sortable_name.clone();
		identity.login_id = user.login_id.clone();
		identity.email = user.email.clone();
	}
	identity
}

/// Every attempt Canvas reported, newest information first resolved into our own shape.
fn attempts_of(
	submission: &CanvasSubmissionPayload,
	downloads: &DownloadedAttachments,
	diagnostics: &mut Vec<InputDiagnostic>,
	identity: &StudentIdentity,
) -> Vec<SubmissionAttempt> {
	let rows: Vec<&CanvasSubmissionPayload> = if submission.submission_history.is_empty() {
		vec![submission]
	} else {
		submission.submission_history.iter().collect()
	};

	let mut attempts: Vec<SubmissionAttempt> = Vec::new();
	for row in rows {
		// No attempt number means Canvas's placeholder row: nothing was handed in.
		let Some(number) = row.attempt else {
			continue;
		};

		let mut attachments = Vec::new();
		let mut files = Vec::new();
		for payload in &row.attachments {
			let name = payload.name();
			attachments.push(Attachment {
				id: payload.id,
				filename: name.clone(),
				content_type: payload.content_type.clone(),
				size: payload.size,
				url: payload.url.clone(),
			});

			match downloads.get(&payload.id) {
				Some(path) => {
					let ext = Path::new(&name)
						.extension()
						.and_then(|e| e.to_str())
						.unwrap_or("")
						.to_lowercase();
					if let Some(language) = detect_language(&ext) {
						files.push(StudentFile::direct(path.clone(), language).with_origin(
							FileOrigin::Attachment {
								attempt: number,
								attachment_id: payload.id,
							},
						));
					} else {
						diagnostics.push(InputDiagnostic::info(DiagnosticKind::IgnoredFile {
							key: identity.key.raw(),
							path: path.clone(),
						}));
					}
				}
				// Not downloaded, so there is nothing to run — the student is not made
				// executable on the strength of an attachment we do not have.
				None => {
					diagnostics.push(InputDiagnostic::warning(DiagnosticKind::PendingDownload {
						attachment_id: payload.id,
						filename: name.clone(),
					}))
				}
			}
		}

		files.sort_by(|a, b| a.path.cmp(&b.path));
		attachments.sort_by(|a, b| a.filename.cmp(&b.filename));

		attempts.push(SubmissionAttempt {
			attempt: number,
			submitted_at: row.submitted_at.clone(),
			source_status: Some(row.source_status()),
			attachments,
			files,
		});
	}

	attempts.sort_by_key(|a| a.attempt);
	attempts
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{StudentKey, SubmissionOutcome};

	fn user(id: u64, sis: Option<&str>, name: &str) -> CanvasUserPayload {
		CanvasUserPayload {
			id,
			name: Some(name.to_string()),
			sortable_name: None,
			sis_user_id: sis.map(str::to_string),
			login_id: None,
			email: None,
		}
	}

	/// Canvas emits `"content-type"`; P-669's fixtures used `"content_type"`. Both have to
	/// load, or either the real API or every existing fixture silently yields `None`.
	#[test]
	fn test_both_content_type_spellings_deserialise() {
		let hyphen: CanvasAttachmentPayload =
			serde_json::from_str(r#"{"id":1,"content-type":"text/x-python"}"#).unwrap();
		assert_eq!(hyphen.content_type.as_deref(), Some("text/x-python"));

		let underscore: CanvasAttachmentPayload =
			serde_json::from_str(r#"{"id":1,"content_type":"text/x-python"}"#).unwrap();
		assert_eq!(underscore.content_type.as_deref(), Some("text/x-python"));

		// A payload that simply omits it is not an error.
		let absent: CanvasAttachmentPayload = serde_json::from_str(r#"{"id":1}"#).unwrap();
		assert_eq!(absent.content_type, None);
	}

	/// A saved bundle is re-read by the same struct, so what we write must be what we can
	/// read back — the whole point of `rename` over a bare `alias`.
	#[test]
	fn test_a_saved_payload_round_trips_its_content_type() {
		let original = attachment(1, "lab1.py");
		let json = serde_json::to_string(&original).unwrap();
		assert!(
			json.contains(r#""content-type""#),
			"the wire spelling is what gets written, got {json}"
		);

		let reloaded: CanvasAttachmentPayload = serde_json::from_str(&json).unwrap();
		assert_eq!(reloaded, original);
		assert_eq!(reloaded.content_type.as_deref(), Some("text/x-python"));
	}

	fn attachment(id: u64, name: &str) -> CanvasAttachmentPayload {
		CanvasAttachmentPayload {
			id,
			filename: Some(name.to_string()),
			display_name: Some(name.to_string()),
			content_type: Some("text/x-python".to_string()),
			size: Some(4),
			url: Some(format!("https://example.invalid/files/{id}")),
		}
	}

	fn submitted(
		user_id: u64,
		attempt: u32,
		attachments: Vec<CanvasAttachmentPayload>,
	) -> CanvasSubmissionPayload {
		CanvasSubmissionPayload {
			id: user_id * 10 + attempt as u64,
			user_id,
			attempt: Some(attempt),
			workflow_state: Some("submitted".to_string()),
			submitted_at: Some(format!("2026-03-0{attempt}T00:00:00Z")),
			late: false,
			missing: false,
			excused: None,
			submission_type: Some("online_upload".to_string()),
			body: None,
			attachments,
			submission_history: Vec::new(),
		}
	}

	fn placeholder(user_id: u64) -> CanvasSubmissionPayload {
		CanvasSubmissionPayload {
			id: 0,
			user_id,
			attempt: None,
			workflow_state: Some("unsubmitted".to_string()),
			submitted_at: None,
			late: false,
			missing: true,
			excused: None,
			submission_type: None,
			body: None,
			attachments: Vec::new(),
			submission_history: Vec::new(),
		}
	}

	fn downloads(pairs: &[(u64, &str)]) -> DownloadedAttachments {
		pairs
			.iter()
			.map(|(id, path)| (*id, PathBuf::from(path)))
			.collect()
	}

	#[test]
	fn test_unsubmitted_placeholder_is_not_submitted_not_empty() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010004"), "Dan")],
			submissions: vec![placeholder(1)],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		assert_eq!(input.student_count(), 1);
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::NotSubmitted);
	}

	#[test]
	fn test_submitted_with_no_attachments_is_empty_not_missing() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010005"), "Eve")],
			submissions: vec![submitted(1, 1, vec![])],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::SubmittedEmpty
		);
	}

	#[test]
	fn test_latest_attempt_wins_whatever_the_history_order() {
		let mut base = submitted(1, 2, vec![attachment(20, "lab1.py")]);
		base.submission_history = vec![
			submitted(1, 2, vec![attachment(20, "lab1.py")]),
			submitted(1, 1, vec![attachment(10, "draft.py")]),
		];
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010002"), "Bob")],
			submissions: vec![base.clone()],
			..Default::default()
		};
		let files = downloads(&[(10, "/tmp/draft.py"), (20, "/tmp/lab1.py")]);

		let input = normalize(&payload, None, &files, AttemptPolicy::Latest);
		let student = &input.students[0];
		assert_eq!(student.selected_attempt().unwrap().attempt, 2);
		assert_eq!(student.files()[0].file_name(), "lab1.py");

		// Reversed history must not change the answer.
		let mut reversed = base;
		reversed.submission_history.reverse();
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010002"), "Bob")],
			submissions: vec![reversed],
			..Default::default()
		};
		let input = normalize(&payload, None, &files, AttemptPolicy::Latest);
		assert_eq!(input.students[0].selected_attempt().unwrap().attempt, 2);
	}

	#[test]
	fn test_undownloaded_attachment_does_not_make_a_student_executable() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![submitted(1, 1, vec![attachment(10, "lab1.py")])],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::SubmittedEmpty
		);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::PendingDownload { .. }))
		);
		// The attachment is still recorded — we know it exists, we just don't have it.
		assert_eq!(input.students[0].artifact_names(), vec!["lab1.py"]);
	}

	#[test]
	fn test_user_without_sis_id_is_kept_under_its_canvas_id() {
		let payload = CanvasPayload {
			users: vec![user(4242, None, "No SIS")],
			submissions: vec![submitted(4242, 1, vec![attachment(10, "lab1.py")])],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/lab1.py")]),
			AttemptPolicy::Latest,
		);

		assert_eq!(input.student_count(), 1);
		assert_eq!(input.students[0].key(), &StudentKey::CanvasUser(4242));
		assert_eq!(input.students[0].identity.key.to_string(), "canvas:4242");
		assert!(input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::MissingStudentNumber { canvas_user_id } if *canvas_user_id == 4242
		)));
		// Enrollment is the roster when none is supplied, and Canvas says this person is
		// enrolled — so they are a member, not a stranger.
		assert_eq!(input.students[0].roster_match, RosterMatch::Matched(0));
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
	}

	#[test]
	fn test_a_sis_id_that_looks_like_a_rendered_key_is_not_used_as_a_number() {
		// Would otherwise render as `local:alice` and parse back as an Extracted key,
		// so Display would stop being reversible.
		let payload = CanvasPayload {
			users: vec![user(7, Some("local:alice"), "Odd")],
			submissions: vec![placeholder(7)],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		assert_eq!(input.students[0].key(), &StudentKey::CanvasUser(7));
		assert!(input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::MissingStudentNumber { canvas_user_id } if *canvas_user_id == 7
		)));
	}

	#[test]
	fn test_numeric_sis_user_id_is_rejected_rather_than_coerced() {
		// A lenient number→string coercion here would silently turn 0024010003 into
		// 24010003 and merge two students.
		let err = serde_json::from_str::<CanvasUserPayload>(r#"{"id":1,"sis_user_id":24010003}"#)
			.unwrap_err();
		assert!(
			err.to_string().contains("invalid type"),
			"expected a type error, got: {err}"
		);
	}

	#[test]
	fn test_canvas_enrollment_outranks_a_stale_supplied_roster() {
		let payload = CanvasPayload {
			users: vec![
				user(1, Some("2024010001"), "Alice"),
				user(2, Some("9999999999"), "Late Add"),
			],
			submissions: vec![
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
				submitted(2, 1, vec![attachment(20, "lab1.py")]),
			],
			..Default::default()
		};
		// The teacher's CSV predates the late enrolment.
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let files = downloads(&[(10, "/tmp/a.py"), (20, "/tmp/b.py")]);

		let input = normalize(&payload, Some(&roster), &files, AttemptPolicy::Latest);

		// Canvas is authoritative about who is in the course, so someone it says is
		// enrolled is a member — not a stranger whose work goes ungraded.
		for key in ["2024010001", "9999999999"] {
			let student = input
				.students
				.iter()
				.find(|s| s.key().raw() == key)
				.unwrap_or_else(|| panic!("{key} missing"));
			assert_eq!(student.outcome(), SubmissionOutcome::Executable, "{key}");
		}
	}

	#[test]
	fn test_a_supplied_row_canvas_has_never_heard_of_is_kept_and_flagged() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![placeholder(1)],
			..Default::default()
		};
		// Somebody the teacher tracks who was never enrolled — kept, so a hand-maintained
		// list cannot silently lose people either.
		let roster = Roster::from_pairs(&[("2024010001", "Alice"), ("2024019999", "Ghost")]);
		let input = normalize(
			&payload,
			Some(&roster),
			&downloads(&[]),
			AttemptPolicy::Latest,
		);

		assert_eq!(input.student_count(), 2);
		let ghost = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024019999")
			.expect("the supplied-only row must survive");
		assert_eq!(ghost.outcome(), SubmissionOutcome::NotSubmitted);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::NotEnrolled { .. })),
			"and must be flagged as not enrolled"
		);
	}

	#[test]
	fn test_enrollment_becomes_the_roster_when_none_is_supplied() {
		let payload = CanvasPayload {
			users: vec![
				user(1, Some("2024010001"), "Alice"),
				user(2, Some("2024010004"), "Dan"),
			],
			submissions: vec![
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
				placeholder(2),
			],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py")]),
			AttemptPolicy::Latest,
		);

		assert_eq!(input.student_count(), 2);
		assert_eq!(input.roster.as_ref().unwrap().len(), 2);
		let dan = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024010004")
			.unwrap();
		assert_eq!(dan.outcome(), SubmissionOutcome::NotSubmitted);
	}

	#[test]
	fn test_two_accounts_claiming_one_student_number_is_a_hard_error() {
		let payload = CanvasPayload {
			users: vec![
				user(1, Some("2024010001"), "Alice"),
				user(2, Some("2024010001"), "Alice Chen"),
			],
			submissions: vec![
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
				submitted(2, 1, vec![attachment(20, "lab1.py")]),
			],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py"), (20, "/tmp/b.py")]),
			AttemptPolicy::Latest,
		);

		// A student number identifies one person, so two enrolments claiming it cannot
		// both be right — and picking one would grade somebody's work under another name.
		let errors: Vec<_> = input.errors().collect();
		assert_eq!(errors.len(), 1);
		assert!(matches!(
			&errors[0].kind,
			DiagnosticKind::ConflictingRosterEntry { key, .. } if key == "2024010001"
		));
	}

	#[test]
	fn test_sis_id_is_trimmed_the_same_way_the_csv_is() {
		let payload = CanvasPayload {
			users: vec![user(1, Some(" 2024010001 "), "Alice")],
			submissions: vec![submitted(1, 1, vec![attachment(10, "lab1.py")])],
			..Default::default()
		};
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let input = normalize(
			&payload,
			Some(&roster),
			&downloads(&[(10, "/tmp/a.py")]),
			AttemptPolicy::Latest,
		);

		assert_eq!(input.students[0].roster_match, RosterMatch::Matched(0));
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
	}

	#[test]
	fn test_a_contradictory_roster_is_reported_the_same_whatever_the_payload_order() {
		let users = vec![
			user(1, Some("2024010001"), "Alice One"),
			user(2, Some("2024010001"), "Alice Two"),
		];
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);

		let run = |users: Vec<CanvasUserPayload>| {
			serde_json::to_string(&normalize(
				&CanvasPayload {
					users,
					..Default::default()
				},
				Some(&roster),
				&downloads(&[]),
				AttemptPolicy::Latest,
			))
			.unwrap()
		};

		let mut reversed = users.clone();
		reversed.reverse();
		// Which account the payload happened to list first must not decide anything.
		assert_eq!(run(users), run(reversed));
	}

	#[test]
	fn test_a_canvas_id_never_resolves_against_someone_elses_student_number() {
		// User 2024010001's Canvas id is the decimal text of another student's 学号.
		let payload = CanvasPayload {
			users: vec![
				user(2024010001, None, "No SIS"),
				user(7, Some("2024010001"), "Alice"),
			],
			submissions: vec![placeholder(2024010001), placeholder(7)],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		let no_sis = input
			.students
			.iter()
			.find(|s| s.key() == &StudentKey::CanvasUser(2024010001))
			.expect("the SIS-less enrollee");
		// Their record must not pick up Alice's identity across the namespace boundary.
		assert_eq!(no_sis.identity.sis_user_id, None);
		assert_eq!(no_sis.identity.name.as_deref(), Some("No SIS"));
	}

	#[test]
	fn test_a_repeated_submission_row_does_not_become_a_second_student() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
			],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py")]),
			AttemptPolicy::Latest,
		);

		assert_eq!(input.student_count(), 1);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::DuplicateSubmissionRow { .. }))
		);
	}

	#[test]
	fn test_text_entry_without_a_file_is_flagged() {
		let mut submission = submitted(1, 1, vec![]);
		submission.submission_type = Some("online_text_entry".to_string());
		submission.body = Some("my answer".to_string());
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010006"), "Faye")],
			submissions: vec![submission],
			..Default::default()
		};
		let input = normalize(&payload, None, &downloads(&[]), AttemptPolicy::Latest);

		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::SubmittedEmpty
		);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::TextEntryOnly { .. }))
		);
	}

	#[test]
	fn test_source_status_is_preserved_but_stays_out_of_the_file_list() {
		let mut submission = submitted(1, 1, vec![attachment(10, "lab1.py")]);
		submission.late = true;
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![submission],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py")]),
			AttemptPolicy::Latest,
		);

		let status = input.students[0]
			.selected_attempt()
			.unwrap()
			.source_status
			.as_ref()
			.unwrap();
		assert!(status.late);
		assert_eq!(status.workflow_state, "submitted");
	}
}
