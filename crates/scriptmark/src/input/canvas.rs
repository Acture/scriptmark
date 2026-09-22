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

/// One attachment that made it onto disk, with whatever an archive expanded to.
///
/// `expanded` is empty for an ordinary file. It is re-derived from the archive on every
/// run rather than recorded in the bundle manifest: flattening loses the in-archive path,
/// so only the zip index can restore `entry`, and a second recorded copy would drift from
/// disk the moment an extraction directory is touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedAttachment {
	pub path: PathBuf,
	pub expanded: Vec<ExpandedEntry>,
}

impl DownloadedAttachment {
	/// An attachment that is the file itself.
	pub fn file(path: impl Into<PathBuf>) -> Self {
		Self {
			path: path.into(),
			expanded: Vec::new(),
		}
	}
}

/// One file lifted out of an archive attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedEntry {
	/// The path *inside* the archive, before flattening.
	pub entry: String,
	pub path: PathBuf,
}

/// What became of each attachment, keyed by attachment id.
///
/// The value is a `Result` so that a download which was *attempted and failed* carries its
/// reason to the one place a student's identity is in scope. Absence keeps its original
/// meaning — never attempted — and the two produce different diagnostics: a teacher needs
/// to know whether a file is missing because Canvas refused it or because the fetch never
/// got that far.
pub type DownloadedAttachments = HashMap<u64, Result<DownloadedAttachment, String>>;

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
	assignment: Assignment,
) -> AssignmentInput {
	let mut diagnostics: Vec<InputDiagnostic> = Vec::new();

	let users: BTreeMap<u64, &CanvasUserPayload> =
		payload.users.iter().map(|u| (u.id, u)).collect();

	let roster = merged_roster(&payload.users, roster, &mut diagnostics);

	let mut students: Vec<StudentSubmission> = Vec::new();
	// Keyed on the value, not its rendering — `Display` prefixes are not escaped.
	let mut covered: std::collections::BTreeSet<StudentKey> = Default::default();
	let mut covered_canvas_ids: std::collections::BTreeSet<u64> = Default::default();

	// Rows are grouped per user *before* anything is decided. Canvas's submissions index is
	// offset-paginated over a relation recomputed per request, so an enrollment landing
	// mid-walk shifts the window and re-reads a row. Keeping whichever copy arrived first
	// keeps the older snapshot — and if the student submitted between the two page fetches,
	// that copy is the placeholder, which would report a real submitter 缺交.
	let mut by_user: BTreeMap<u64, Vec<&CanvasSubmissionPayload>> = BTreeMap::new();
	for submission in &payload.submissions {
		by_user
			.entry(submission.user_id)
			.or_default()
			.push(submission);
	}

	// Placeholder rows are kept rather than dropped: they carry the record's `excused`, and
	// 免交 lands on exactly such a row — a teacher excuses a student who never submitted, so
	// there is no attempt for the status to live on.
	let mut placeholders: BTreeMap<u64, &CanvasSubmissionPayload> = BTreeMap::new();

	for (user_id, rows) in &by_user {
		let submission = richest_row(rows);
		if rows.len() > 1 {
			diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::DuplicateSubmissionRow {
					canvas_user_id: *user_id,
					kept: submission.attempt,
				},
			));
		}

		let user = users.get(user_id).copied();
		let mut identity = identity_for(*user_id, user, &mut diagnostics);

		let attempts = attempts_of(submission, downloads, &mut diagnostics, &identity);

		// A placeholder row for someone the roster covers is handled by the roster merge
		// below, which is the only producer of NotSubmitted. A placeholder row for someone
		// the roster does not cover describes a non-event: nothing arrived and nobody is
		// owed a grade.
		if attempts.is_empty() {
			placeholders.insert(*user_id, submission);
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

		students.push(
			StudentSubmission::received(identity, roster_match, attempts, policy)
				.with_record_status(Some(submission.source_status())),
		);
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
		// 免交 rides on the placeholder row, which has no attempt to hang it on.
		let record_status = identity
			.canvas_user_id
			.and_then(|id| placeholders.get(&id))
			.map(|row| row.source_status());
		students.push(StudentSubmission::not_submitted(
			identity,
			index,
			record_status,
		));
	}

	diagnostics.extend(roster.diagnostics.iter().cloned());
	// Sorted first: `dedup` folds only *consecutive* duplicates, and `attempts_of` iterates
	// attempt-major over attachment-minor, so the repeated pushes an attachment carried
	// forward across attempts generates are never adjacent.
	diagnostics.sort();
	diagnostics.dedup();

	// The declared assignment wins; the payload only fills what it left unset. Dropping it
	// here is what would silently discard a teacher's declared items — and, with the policy
	// that travels beside it, grade the wrong attempt.
	let assignment = Assignment {
		name: if assignment.name.is_empty() {
			payload.assignment_name.clone().unwrap_or_default()
		} else {
			assignment.name
		},
		canvas_course_id: assignment.canvas_course_id.or(payload.course_id),
		canvas_assignment_id: assignment.canvas_assignment_id.or(payload.assignment_id),
		..assignment
	};

	let mut input = AssignmentInput {
		assignment,
		source: InputSource::Canvas {
			course_id: payload.course_id,
			assignment_id: payload.assignment_id,
		},
		roster: Some(roster),
		students,
		attempt_policy: policy,
		unmatched: Vec::new(),
		diagnostics,
	};

	let zero_padded = input.detect_zero_padded_variants();
	input.diagnostics.extend(zero_padded);

	input.sorted()
}

/// Which of several rows for one user to believe.
///
/// Total by construction: a row with an attempt beats one without, a higher attempt beats a
/// lower one, and two placeholders carry the same information so either will do. `rows` is
/// never empty — it comes from a map entry that was created by pushing into it.
fn richest_row<'a>(rows: &[&'a CanvasSubmissionPayload]) -> &'a CanvasSubmissionPayload {
	rows.iter()
		.copied()
		.max_by_key(|row| (row.attempt.is_some(), row.attempt.unwrap_or(0)))
		.expect("a grouped entry always holds at least one row")
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

/// The language of a file, by extension, or `None` when nothing here runs it.
fn language_of(path: &Path) -> Option<&'static str> {
	let ext = path
		.extension()
		.and_then(|e| e.to_str())
		.unwrap_or("")
		.to_lowercase();
	detect_language(&ext)
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

		// A default arm, not a list of known-bad types: an enumeration leaves anything
		// unlisted — `basic_lti_launch`, or whatever Canvas adds next — with no explanation
		// at all, and the ticket asks for the specific reason. `online_text_entry` keeps its
		// more precise `TextEntryOnly`.
		match row.submission_type.as_deref() {
			None | Some("online_upload") | Some("online_text_entry") => {}
			Some(other) => diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::UnsupportedSubmissionType {
					key: identity.key.raw(),
					submission_type: other.to_string(),
				},
			)),
		}

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
				// An archive was expanded, so its entries are the candidates — the zip
				// itself runs nothing. Language is detected from the entry name, and the
				// in-archive path is kept so a file can be traced back to what it arrived
				// in.
				Some(Ok(downloaded)) if !downloaded.expanded.is_empty() => {
					for expanded in &downloaded.expanded {
						match language_of(&expanded.path) {
							Some(language) => files.push(
								StudentFile::direct(expanded.path.clone(), language).with_origin(
									FileOrigin::Attachment {
										attempt: number,
										attachment_id: payload.id,
										entry: Some(expanded.entry.clone()),
									},
								),
							),
							None => diagnostics.push(InputDiagnostic::info(
								DiagnosticKind::IgnoredFile {
									key: identity.key.raw(),
									path: expanded.path.clone(),
								},
							)),
						}
					}
				}
				Some(Ok(downloaded)) => match language_of(Path::new(&name)) {
					Some(language) => files.push(
						StudentFile::direct(downloaded.path.clone(), language).with_origin(
							FileOrigin::Attachment {
								attempt: number,
								attachment_id: payload.id,
								entry: None,
							},
						),
					),
					None => diagnostics.push(InputDiagnostic::info(DiagnosticKind::IgnoredFile {
						key: identity.key.raw(),
						path: downloaded.path.clone(),
					})),
				},
				// Attempted and refused. Named, and attributed, so the failure list tells a
				// teacher who lost work — and so this is never mistaken for 缺交.
				Some(Err(reason)) => diagnostics.push(InputDiagnostic::warning(
					DiagnosticKind::AttachmentUnavailable {
						key: identity.key.raw(),
						attachment_id: payload.id,
						filename: name.clone(),
						reason: reason.clone(),
					},
				)),
				// Never attempted, so there is nothing to run — the student is not made
				// executable on the strength of an attachment we do not have.
				None => {
					diagnostics.push(InputDiagnostic::warning(DiagnosticKind::PendingDownload {
						key: identity.key.raw(),
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
			.map(|(id, path)| (*id, Ok(DownloadedAttachment::file(*path))))
			.collect()
	}

	/// 免交 lands on a row with no attempt: a teacher excuses a student who never handed
	/// anything in, so `attempt` stays null and there is no attempt for the status to live
	/// on. Reading it off an attempt would make them indistinguishable from someone who
	/// simply forgot.
	#[test]
	fn test_an_excused_non_submitter_is_distinguishable_from_a_forgetful_one() {
		let mut excused = placeholder(1);
		excused.excused = Some(true);
		excused.workflow_state = Some("graded".to_string());

		let payload = CanvasPayload {
			users: vec![
				user(1, Some("2024010001"), "Alice"),
				user(2, Some("2024010002"), "Bob"),
			],
			submissions: vec![excused, placeholder(2)],
			..CanvasPayload::default()
		};

		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

		let alice = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024010001")
			.expect("alice");
		let bob = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024010002")
			.expect("bob");

		// Both are 缺交 as far as delivery goes — the difference is why.
		assert_eq!(alice.outcome(), SubmissionOutcome::NotSubmitted);
		assert_eq!(bob.outcome(), SubmissionOutcome::NotSubmitted);
		assert!(alice.is_excused(), "the excusal must survive normalisation");
		assert!(!bob.is_excused());
	}

	/// Excused is a property of the submission record, not of an attempt. Reading it off
	/// the *selected* attempt reports an excusal applied after that attempt as absent —
	/// which is exactly what an `earliest` policy selects.
	#[test]
	fn test_an_excusal_survives_an_earliest_attempt_policy() {
		let mut row = submitted(1, 2, vec![attachment(20, "lab1.py")]);
		row.excused = Some(true);
		row.submission_history = vec![
			submitted(1, 1, vec![attachment(10, "lab1.py")]),
			submitted(1, 2, vec![attachment(20, "lab1.py")]),
		];

		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![row],
			..CanvasPayload::default()
		};

		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py"), (20, "/tmp/b.py")]),
			AttemptPolicy::Earliest,
			Assignment::default(),
		);

		let alice = &input.students[0];
		// The graded attempt is the first one...
		assert_eq!(alice.selected_attempt().map(|a| a.attempt), Some(1));
		// ...but the excusal belongs to the record, and is still visible.
		assert!(alice.is_excused());
	}

	/// Canvas's submissions index is offset-paginated over a relation recomputed per
	/// request, so a row can be read twice. Keeping the first copy keeps the older
	/// snapshot — and if the student submitted in between, that copy is the placeholder.
	#[test]
	fn test_a_duplicate_row_keeps_the_richer_copy_not_the_first() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			// The placeholder is listed first, as the earlier page would have carried it.
			submissions: vec![
				placeholder(1),
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
			],
			..CanvasPayload::default()
		};

		let input = normalize(
			&payload,
			None,
			&downloads(&[(10, "/tmp/a.py")]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

		assert_eq!(input.student_count(), 1);
		// The submitter is not reported 缺交.
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
		assert!(input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::DuplicateSubmissionRow { kept: Some(1), .. }
		)));
	}

	/// An enumerated list of unsupported types leaves anything unlisted with no explanation
	/// at all. The ticket asks for the specific reason, so the rule is a default arm.
	#[test]
	fn test_an_unknown_submission_type_still_gets_a_reason() {
		let mut row = submitted(1, 1, vec![]);
		row.submission_type = Some("basic_lti_launch".to_string());

		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![row],
			..CanvasPayload::default()
		};

		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

		assert!(
			input.diagnostics.iter().any(|d| matches!(
				&d.kind,
				DiagnosticKind::UnsupportedSubmissionType { submission_type, key }
					if submission_type == "basic_lti_launch" && key == "2024010001"
			)),
			"got {:?}",
			input.diagnostics
		);
	}

	/// A download that was attempted and refused is not the same fact as one that was never
	/// attempted, and neither may look like 缺交.
	#[test]
	fn test_a_failed_download_names_the_student_and_is_not_a_non_submission() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![submitted(
				1,
				1,
				vec![attachment(10, "lab1.py"), attachment(20, "extra.py")],
			)],
			..CanvasPayload::default()
		};

		let downloads: DownloadedAttachments = HashMap::from([
			(10, Ok(DownloadedAttachment::file("/tmp/a.py"))),
			(20, Err("502 Bad Gateway".to_string())),
		]);

		let input = normalize(
			&payload,
			None,
			&downloads,
			AttemptPolicy::Latest,
			Assignment::default(),
		);

		let alice = &input.students[0];
		// They kept what did arrive, and are emphatically not 缺交.
		assert_eq!(alice.outcome(), SubmissionOutcome::Executable);
		assert_eq!(alice.files().len(), 1);
		assert!(
			input.diagnostics.iter().any(|d| matches!(
				&d.kind,
				DiagnosticKind::AttachmentUnavailable { key, attachment_id, reason, .. }
					if key == "2024010001" && *attachment_id == 20 && reason.contains("502")
			)),
			"the failure list must name who lost work, got {:?}",
			input.diagnostics
		);
	}

	/// `Vec::dedup` folds only *consecutive* duplicates, and `attempts_of` iterates
	/// attempt-major over attachment-minor — so two carried-forward attachments across two
	/// attempts push A1 B1 A2 B2, with no two equal entries adjacent.
	#[test]
	fn test_repeated_diagnostics_are_folded_across_attempts() {
		let carried = vec![attachment(10, "a.py"), attachment(20, "b.py")];
		let mut row = submitted(1, 2, carried.clone());
		row.submission_history = vec![submitted(1, 1, carried.clone()), submitted(1, 2, carried)];

		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![row],
			..CanvasPayload::default()
		};

		// Neither is downloaded, so each pushes one PendingDownload per attempt.
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

		let pending = input
			.diagnostics
			.iter()
			.filter(|d| matches!(&d.kind, DiagnosticKind::PendingDownload { .. }))
			.count();
		assert_eq!(pending, 2, "one line per file, not per file per attempt");
	}

	/// The declared assignment wins; the payload fills only what it left unset. Dropping it
	/// would silently discard the teacher's declared items.
	#[test]
	fn test_the_declared_assignment_survives_normalisation() {
		let payload = CanvasPayload {
			assignment_name: Some("Canvas HW1".to_string()),
			course_id: Some(7),
			assignment_id: Some(9),
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![placeholder(1)],
		};

		let declared = Assignment {
			name: "Lab 1".to_string(),
			..Assignment::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Earliest,
			declared,
		);

		assert_eq!(input.assignment.name, "Lab 1");
		// The ids the toml did not carry come from the payload.
		assert_eq!(input.assignment.canvas_course_id, Some(7));
		assert_eq!(input.assignment.canvas_assignment_id, Some(9));
		// And the rule that chose every `selected` is recorded beside them.
		assert_eq!(input.attempt_policy, AttemptPolicy::Earliest);
	}

	#[test]
	fn test_unsubmitted_placeholder_is_not_submitted_not_empty() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010004"), "Dan")],
			submissions: vec![placeholder(1)],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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

		let input = normalize(
			&payload,
			None,
			&files,
			AttemptPolicy::Latest,
			Assignment::default(),
		);
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
		let input = normalize(
			&payload,
			None,
			&files,
			AttemptPolicy::Latest,
			Assignment::default(),
		);
		assert_eq!(input.students[0].selected_attempt().unwrap().attempt, 2);
	}

	#[test]
	fn test_undownloaded_attachment_does_not_make_a_student_executable() {
		let payload = CanvasPayload {
			users: vec![user(1, Some("2024010001"), "Alice")],
			submissions: vec![submitted(1, 1, vec![attachment(10, "lab1.py")])],
			..Default::default()
		};
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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
			Assignment::default(),
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
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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

		let input = normalize(
			&payload,
			Some(&roster),
			&files,
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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
			Assignment::default(),
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
			Assignment::default(),
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
			Assignment::default(),
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
			Assignment::default(),
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
				Assignment::default(),
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
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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
			Assignment::default(),
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
		let input = normalize(
			&payload,
			None,
			&downloads(&[]),
			AttemptPolicy::Latest,
			Assignment::default(),
		);

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
			Assignment::default(),
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
