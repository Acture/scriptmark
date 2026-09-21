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
	StudentSubmission, SubmissionAttempt, normalize_key,
};
use crate::roster::{Roster, RosterEntry, RosterLookup};

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

/// An uploaded file, as Canvas reports it on a submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasAttachmentPayload {
	pub id: u64,
	#[serde(default)]
	pub filename: Option<String>,
	#[serde(default)]
	pub display_name: Option<String>,
	#[serde(default)]
	pub content_type: Option<String>,
	#[serde(default)]
	pub size: Option<u64>,
	#[serde(default)]
	pub url: Option<String>,
}

impl CanvasAttachmentPayload {
	fn name(&self) -> String {
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
/// When a roster is supplied it is the roster of record: Canvas users only enrich identity
/// (name, SIS id, login id) and never add or remove membership. Without one, course
/// enrollment *is* the roster — Canvas genuinely knows who is enrolled — so a non-submitter
/// still appears rather than vanishing.
pub fn normalize(
	payload: &CanvasPayload,
	roster: Option<&Roster>,
	downloads: &DownloadedAttachments,
	policy: AttemptPolicy,
) -> AssignmentInput {
	let mut diagnostics: Vec<InputDiagnostic> = Vec::new();

	let users: BTreeMap<u64, &CanvasUserPayload> =
		payload.users.iter().map(|u| (u.id, u)).collect();

	let roster = match roster {
		Some(roster) => roster.clone(),
		None => enrollment_roster(&payload.users),
	};

	let mut students: Vec<StudentSubmission> = Vec::new();
	let mut covered: std::collections::BTreeSet<String> = Default::default();

	// Deterministic regardless of payload order.
	let mut submissions: Vec<&CanvasSubmissionPayload> = payload.submissions.iter().collect();
	submissions.sort_by_key(|s| s.user_id);

	for submission in submissions {
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

		let roster_match = match identity.student_number.as_deref() {
			None => {
				diagnostics.push(InputDiagnostic::warning(DiagnosticKind::NotOnRoster {
					key: identity.key.raw(),
				}));
				RosterMatch::NotInRoster
			}
			Some(number) => match roster.lookup(number) {
				RosterLookup::Unique(i) => {
					covered.insert(number.to_string());
					if identity.name.is_none() {
						identity.name = roster.entries[i].name.clone();
					}
					RosterMatch::Matched(i)
				}
				RosterLookup::Ambiguous(hits) => {
					covered.insert(number.to_string());
					diagnostics.push(InputDiagnostic::warning(
						DiagnosticKind::AmbiguousRosterMatch {
							key: number.to_string(),
							count: hits.len(),
						},
					));
					RosterMatch::Ambiguous(hits)
				}
				RosterLookup::Missing => {
					diagnostics.push(InputDiagnostic::warning(DiagnosticKind::NotOnRoster {
						key: number.to_string(),
					}));
					RosterMatch::NotInRoster
				}
			},
		};

		students.push(StudentSubmission::received(
			identity,
			roster_match,
			attempts,
			policy,
		));
	}

	for (i, entry) in roster.entries.iter().enumerate() {
		if covered.contains(&entry.student_number) {
			continue;
		}
		let mut identity = StudentIdentity::number(&entry.student_number);
		identity.name = entry.name.clone();
		identity.canvas_user_id = entry.canvas_user_id;
		students.push(StudentSubmission::not_submitted(identity, i));
	}

	diagnostics.extend(roster.diagnostics.iter().cloned());

	let mut input = AssignmentInput {
		assignment: Assignment {
			name: payload.assignment_name.clone().unwrap_or_default(),
			canvas_course_id: payload.course_id,
			canvas_assignment_id: payload.assignment_id,
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

/// Build a roster from course enrollment, for the case where the teacher supplied none.
fn enrollment_roster(users: &[CanvasUserPayload]) -> Roster {
	let mut entries: Vec<RosterEntry> = users
		.iter()
		.filter_map(|u| {
			let number = normalize_key(u.sis_user_id.as_deref()?);
			(!number.is_empty()).then(|| RosterEntry {
				student_number: number,
				name: u.name.clone(),
				canvas_user_id: Some(u.id),
				location: None,
			})
		})
		.collect();
	entries.sort_by(|a, b| a.student_number.cmp(&b.student_number));
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
		.filter(|s| !s.is_empty());

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

	let _ = identity;
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
		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::ReceivedUnmatched
		);
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
	fn test_supplied_roster_is_the_roster_of_record() {
		let payload = CanvasPayload {
			users: vec![
				user(1, Some("2024010001"), "Alice"),
				user(2, Some("9999999999"), "Stranger"),
			],
			submissions: vec![
				submitted(1, 1, vec![attachment(10, "lab1.py")]),
				submitted(2, 1, vec![attachment(20, "lab1.py")]),
			],
			..Default::default()
		};
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let files = downloads(&[(10, "/tmp/a.py"), (20, "/tmp/b.py")]);

		let input = normalize(&payload, Some(&roster), &files, AttemptPolicy::Latest);

		let alice = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024010001")
			.unwrap();
		assert_eq!(alice.outcome(), SubmissionOutcome::Executable);
		// Enrolled in Canvas, absent from the roster of record.
		let stranger = input
			.students
			.iter()
			.find(|s| s.key().raw() == "9999999999")
			.unwrap();
		assert_eq!(stranger.outcome(), SubmissionOutcome::ReceivedUnmatched);
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
	fn test_two_users_sharing_a_sis_id_are_both_retained() {
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

		// A map keyed on the student number would have kept one of these.
		assert_eq!(input.student_count(), 2);
		let canvas_ids: Vec<Option<u64>> = input
			.students
			.iter()
			.map(|s| s.identity.canvas_user_id)
			.collect();
		assert_eq!(canvas_ids, vec![Some(1), Some(2)]);
		assert!(input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::DuplicateRosterEntry { count, .. } if *count == 2
		)));
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
