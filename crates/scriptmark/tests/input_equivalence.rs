//! The same assignment, carried through both entry points, must arrive as the same input.
//!
//! A bare `assert_eq!(local, canvas)` would be satisfied by two empty vectors — which is
//! exactly what a gitignored fixture tree produces in CI. So each side is first checked
//! against a hand-written expected table, then the two are compared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::input::canvas::{CanvasPayload, DownloadedAttachments, normalize};
use scriptmark::models::{
	Assignment, AssignmentInput, AttemptPolicy, DiagnosticKind, ProjectedStudent, RosterMatch,
	StudentReport, SubmissionOutcome, UnmatchedReason,
};
use scriptmark::roster::{Roster, load_roster};

fn fixture_root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hw1")
}

/// Copy the committed fixture somewhere writable. The local adapter expands archives beside
/// the directory it scans, and `#[test]` bodies run in parallel — neither belongs in the
/// source tree.
fn copy_dir(src: &Path, dst: &Path) {
	std::fs::create_dir_all(dst).unwrap();
	for entry in std::fs::read_dir(src).unwrap() {
		let entry = entry.unwrap();
		let target = dst.join(entry.file_name());
		if entry.path().is_dir() {
			copy_dir(&entry.path(), &target);
		} else {
			std::fs::copy(entry.path(), &target).unwrap();
		}
	}
}

fn roster() -> Roster {
	let path = fixture_root().join("local/roster.csv");
	load_roster(&path)
		.unwrap_or_else(|e| panic!("fixture roster missing at {}: {e}", path.display()))
}

fn local_input(dir: &tempfile::TempDir) -> AssignmentInput {
	copy_dir(&fixture_root().join("local"), dir.path());
	let roster = roster();
	load_local_input(
		&[dir.path().join("submissions")],
		LocalInputOptions {
			assignment: Assignment::named("hw1"),
			roster: Some(&roster),
			attempt_policy: AttemptPolicy::Latest,
		},
	)
	.unwrap()
}

fn canvas_input() -> AssignmentInput {
	let root = fixture_root().join("canvas");
	let raw = std::fs::read_to_string(root.join("assignment.json"))
		.unwrap_or_else(|e| panic!("fixture payload missing: {e}"));
	let payload: CanvasPayload = serde_json::from_str(&raw).expect("fixture payload must parse");

	// Standing in for P-670's downloader: the files these attachments would land in.
	let downloads: DownloadedAttachments = HashMap::from([
		(1001, root.join("files/1001/lab1.py")),
		(1002, root.join("files/1002/draft.py")),
		(1003, root.join("files/1003/lab1.py")),
		(1004, root.join("files/1004/lab1.py")),
		(1005, root.join("files/1005/lab1.py")),
	]);
	for path in downloads.values() {
		assert!(
			path.is_file(),
			"fixture download missing: {}",
			path.display()
		);
	}

	normalize(&payload, Some(&roster()), &downloads, AttemptPolicy::Latest)
}

/// What both sources must produce, written out rather than derived.
fn expected() -> Vec<ProjectedStudent> {
	use SubmissionOutcome::*;
	let rows: &[(&str, SubmissionOutcome, &[&str])] = &[
		// Two roster rows carry this number; the submitter matches both.
		("2024010001", Executable, &["lab1.py"]),
		// Resubmitted — the second attempt is the one that counts.
		("2024010002", Executable, &["lab1.py"]),
		// Differs from the next row only by zero padding, and stays a separate student.
		("0024010003", Executable, &["lab1.py"]),
		("24010003", Executable, &["lab1.py"]),
		// On the roster, nothing received.
		("2024010004", NotSubmitted, &[]),
		// Something arrived, nothing runnable in it.
		("2024010005", SubmittedEmpty, &[]),
	];
	let mut expected: Vec<ProjectedStudent> = rows
		.iter()
		.map(|(key, outcome, artifacts)| ProjectedStudent {
			key: (*key).to_string(),
			outcome: *outcome,
			artifacts: artifacts.iter().map(|a| a.to_string()).collect(),
		})
		.collect();
	expected.sort();
	expected
}

/// Strip the student-number prefix the local fixture deliberately carries.
///
/// A Canvas attachment is named by the student (`lab1.py`); a teacher's bulk download is
/// named `{sid}_lab1.py`. That is the one shape difference between the sources, and
/// reversing it is a matching rule — which is P-673 — so it lives here and not in the model.
fn canonical(projection: Vec<ProjectedStudent>) -> Vec<ProjectedStudent> {
	projection
		.into_iter()
		.map(|mut student| {
			let prefix = format!("{}_", student.key);
			student.artifacts = student
				.artifacts
				.iter()
				.map(|name| name.strip_prefix(&prefix).unwrap_or(name).to_string())
				.collect();
			student
		})
		.collect()
}

#[test]
fn test_local_material_matches_the_expected_table() {
	let dir = tempfile::tempdir().unwrap();
	let input = local_input(&dir);

	assert_eq!(input.student_count(), 6);
	assert_eq!(canonical(input.projection()), expected());
}

#[test]
fn test_canvas_material_matches_the_expected_table() {
	let input = canvas_input();

	assert_eq!(input.student_count(), 6);
	assert_eq!(canonical(input.projection()), expected());
}

#[test]
fn test_both_entry_points_produce_equivalent_input() {
	let dir = tempfile::tempdir().unwrap();
	let local = local_input(&dir);
	let canvas = canvas_input();

	// Neither side may be trivially empty — that is how this assertion goes vacuous.
	assert_eq!(local.student_count(), 6);
	assert_eq!(canvas.student_count(), 6);
	assert_eq!(
		canonical(local.projection()),
		canonical(canvas.projection())
	);
}

#[test]
fn test_every_roster_member_is_present_on_both_sides() {
	let dir = tempfile::tempdir().unwrap();
	let roster = roster();
	// Seven rows, six distinct numbers: 2024010001 appears twice.
	assert_eq!(roster.len(), 7);

	for input in [local_input(&dir), canvas_input()] {
		for entry in &roster.entries {
			assert!(
				input.students.iter().any(|s| s.identity.key == entry.key),
				"roster member {} vanished",
				entry.key
			);
		}
	}
}

#[test]
fn test_duplicate_roster_rows_are_reported_on_both_sides() {
	let dir = tempfile::tempdir().unwrap();
	for input in [local_input(&dir), canvas_input()] {
		assert!(
			input.diagnostics.iter().any(|d| matches!(
				&d.kind,
				DiagnosticKind::DuplicateRosterEntry { key, count }
					if key == "2024010001" && *count == 2
			)),
			"duplicate roster rows must be reported"
		);
		// Either way it is one student, never one per row.
		assert_eq!(
			input
				.students
				.iter()
				.filter(|s| s.identity.key.raw() == "2024010001")
				.count(),
			1
		);
	}
}

#[test]
fn test_the_duplicate_is_ambiguous_locally_and_resolved_by_canvas() {
	let dir = tempfile::tempdir().unwrap();
	let alice = |input: &AssignmentInput| {
		input
			.students
			.iter()
			.find(|s| s.identity.key.raw() == "2024010001")
			.unwrap()
			.roster_match
			.clone()
	};

	// Locally the CSV is all there is, so both candidate rows are kept and nothing picks
	// one silently.
	assert_eq!(
		alice(&local_input(&dir)),
		RosterMatch::Ambiguous(vec![0, 1])
	);
	// Canvas is authoritative about enrollment and lists this student once, so the CSV's
	// duplicated row is a data-entry error there — reported, but not the roster of record.
	assert_eq!(alice(&canvas_input()), RosterMatch::Matched(1));
}

#[test]
fn test_zero_padded_pair_is_flagged_on_both_sides() {
	let dir = tempfile::tempdir().unwrap();
	for input in [local_input(&dir), canvas_input()] {
		assert!(
			input.diagnostics.iter().any(|d| matches!(
				&d.kind,
				DiagnosticKind::SuspectedZeroPaddedVariant { keys }
					if keys == &["0024010003".to_string(), "24010003".to_string()]
			)),
			"the zero-padded pair must be flagged"
		);
	}
}

#[test]
fn test_resubmission_selects_the_later_attempt_on_the_canvas_side() {
	let canvas = canvas_input();
	let bob = canvas
		.students
		.iter()
		.find(|s| s.identity.key.raw() == "2024010002")
		.unwrap();

	// Asserted directly, not through the projection: a resubmission usually carries the
	// same filename, so picking attempt 1 would project identically and go unnoticed.
	assert_eq!(bob.attempts.len(), 2);
	assert_eq!(bob.selected_attempt().unwrap().attempt, 2);
	assert!(bob.files()[0].path.ends_with("1003/lab1.py"));
	// Canvas-only source state is preserved, and stays off the file list.
	assert!(
		bob.selected_attempt()
			.unwrap()
			.source_status
			.as_ref()
			.unwrap()
			.late
	);
}

/// Each source reaches "received but unmatchable" by a different route, so it is asserted
/// per source rather than across them: locally a filename token no roster confirms, and on
/// Canvas a submission from somebody the course does not list.
#[test]
fn test_canvas_material_from_someone_not_enrolled_is_unmatched() {
	let root = fixture_root().join("canvas");
	let raw = std::fs::read_to_string(root.join("assignment.json")).unwrap();
	let mut payload: CanvasPayload = serde_json::from_str(&raw).unwrap();

	// Submitted, then dropped the course — Canvas no longer lists them.
	payload.submissions.push(
		serde_json::from_str(
			r#"{"id": 9107, "user_id": 107, "attempt": 1, "workflow_state": "submitted",
			    "submitted_at": "2026-03-01T13:00:00Z", "submission_type": "online_upload",
			    "attachments": [{"id": 1006, "filename": "lab1.py", "display_name": "lab1.py"}]}"#,
		)
		.unwrap(),
	);
	let downloads: DownloadedAttachments = HashMap::from([(1006, root.join("files/1001/lab1.py"))]);

	let input = normalize(&payload, Some(&roster()), &downloads, AttemptPolicy::Latest);
	let stranger = input
		.students
		.iter()
		.find(|s| s.identity.canvas_user_id == Some(107))
		.expect("their work must not be discarded");
	assert_eq!(stranger.outcome(), SubmissionOutcome::ReceivedUnmatched);
}

#[test]
fn test_only_the_local_side_can_have_orphan_files() {
	let dir = tempfile::tempdir().unwrap();
	let local = local_input(&dir);

	assert_eq!(local.unmatched.len(), 1);
	assert!(local.unmatched[0].path.ends_with("_scratch_v2.py"));
	assert_eq!(local.unmatched[0].reason, UnmatchedReason::NoStudentKey);

	// Canvas attributes every attachment to a user, so it has no orphan class at all.
	assert!(canvas_input().unmatched.is_empty());
}

#[test]
fn test_assignment_identity_is_kept_apart_from_student_identity() {
	let canvas = canvas_input();
	assert_eq!(canvas.assignment.canvas_course_id, Some(5510));
	assert_eq!(canvas.assignment.canvas_assignment_id, Some(88120));

	let alice = canvas
		.students
		.iter()
		.find(|s| s.identity.key.raw() == "2024010001")
		.unwrap();
	// 学号, Canvas user id, SIS id and login id are four separate fields.
	assert_eq!(alice.identity.student_number.as_deref(), Some("2024010001"));
	assert_eq!(alice.identity.canvas_user_id, Some(101));
	assert_eq!(alice.identity.sis_user_id.as_deref(), Some("2024010001"));
	assert_eq!(alice.identity.login_id.as_deref(), Some("awu"));

	// A non-submitter keeps theirs too, although the teacher's CSV has no Canvas column —
	// it is backfilled from enrollment rather than lost.
	let dan = canvas
		.students
		.iter()
		.find(|s| s.identity.key.raw() == "2024010004")
		.unwrap();
	assert_eq!(dan.outcome(), SubmissionOutcome::NotSubmitted);
	assert_eq!(dan.identity.canvas_user_id, Some(105));
}

#[test]
fn test_local_scan_leaves_no_artifacts_in_the_committed_fixture() {
	let dir = tempfile::tempdir().unwrap();
	let _ = local_input(&dir);
	assert!(
		!fixture_root()
			.join("local/submissions/.scriptmark_extracted")
			.exists(),
		"the committed fixture tree must not be written to"
	);
}

#[test]
fn test_local_input_is_byte_identical_across_runs() {
	let first = tempfile::tempdir().unwrap();
	let second = tempfile::tempdir().unwrap();

	let a = serde_json::to_string(&local_input(&first)).unwrap();
	let b = serde_json::to_string(&local_input(&second)).unwrap();
	// Paths differ between the two temp dirs; everything else must not.
	let normalise =
		|s: String, dir: &tempfile::TempDir| s.replace(dir.path().to_str().unwrap(), "ROOT");
	assert_eq!(normalise(a, &first), normalise(b, &second));
}

#[test]
fn test_results_written_before_this_model_still_load() {
	let raw = std::fs::read_to_string(fixture_root().join("legacy_results.json")).unwrap();
	let reports: Vec<StudentReport> = serde_json::from_str(&raw).expect("legacy results must load");

	assert_eq!(reports.len(), 2);
	for report in &reports {
		// A record that never carried a submission state must not claim one.
		assert!(report.submission_state.is_none());
		assert!(report.canvas_user_id.is_none());
		// And it is still graded exactly as it was.
		assert!(report.is_gradeable());
	}
	assert_eq!(reports[0].student_id, "alice");
	assert_eq!(reports[0].final_grade, Some(95.0));
}
