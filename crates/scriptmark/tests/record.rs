//! P-678: the grading record. `grade` and `run` write one; every consumer reads a revision of
//! it; `rescore` adds a revision from the saved evidence without running anything, and
//! refuses evidence whose tests, submissions or matching have changed.

use std::path::Path;
use std::process::{Command, Output};

use scriptmark::record::Record;

const SPEC: &str = r#"
[meta]
name = "sum"
file = "sum.py"
function = "add"
language = "python"
imports = ["teacher/support.py"]

[[cases]]
name = "small"
args = [1, 2]
expect = 3
check = { function = "same" }

[[cases]]
name = "negative"
args = [-1, -2]
expect = -3
"#;

const ASSIGNMENT: &str = r#"
[assignment]
name = "sums"

[[items]]
id = "sum"
points = 10
aggregation = "proportional"
"#;

/// alice is right; bob adds absolute values, so he passes one case of two.
fn bench() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	for (name, content) in [
		("tests/test_sum.toml", SPEC),
		// The checker's helper is imported, not declared: it still decides the grade.
		(
			"tests/teacher/support.py",
			"from helper import equal\n\ndef same(result, expected):\n    return equal(result, expected)\n",
		),
		(
			"tests/teacher/helper.py",
			"def equal(a, b):\n    return a == b\n",
		),
		("assignment.toml", ASSIGNMENT),
		(
			"submissions/alice_sum.py",
			"def add(a, b):\n    return a + b\n",
		),
		(
			"submissions/bob_sum.py",
			"def add(a, b):\n    return abs(a) + abs(b)\n",
		),
	] {
		write(dir.path(), name, content);
	}
	dir
}

fn write(dir: &Path, name: &str, content: &str) {
	let path = dir.join(name);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, content).unwrap();
}

fn scriptmark(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.args(args)
		.output()
		.unwrap()
}

/// `rescore` with no interpreter anywhere on PATH: it must not need one.
fn rescore(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.env("PATH", "")
		.args(["rescore", "out/results.json"])
		.args(args)
		.output()
		.unwrap()
}

fn text(output: &Output) -> String {
	format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	)
}

fn succeeded(output: Output) -> Output {
	assert!(output.status.success(), "{}", text(&output));
	output
}

fn refused(output: Output, why: &str) {
	assert!(
		!output.status.success(),
		"expected a refusal: {}",
		text(&output)
	);
	assert!(
		text(&output).contains(why),
		"no '{why}' in:\n{}",
		text(&output)
	);
}

const GRADE: [&str; 6] = [
	"grade",
	"submissions",
	"-t",
	"tests",
	"-o",
	"out/results.json",
];

fn record(dir: &Path) -> Record {
	Record::load(&dir.join("out/results.json")).unwrap_or_else(|e| panic!("{e}"))
}

/// Each student's final grade as revision `n` scored it.
fn grades(dir: &Path, n: Option<u32>) -> Vec<(String, Option<f64>)> {
	record(dir)
		.view(n)
		.unwrap()
		.reports
		.iter()
		.map(|r| (r.student_id.clone(), r.final_grade()))
		.collect()
}

fn graded(alice: f64, bob: f64) -> Vec<(String, Option<f64>)> {
	vec![
		("local:alice".into(), Some(alice)),
		("local:bob".into(), Some(bob)),
	]
}

fn all_or_nothing(dir: &Path) {
	write(
		dir,
		"assignment.toml",
		&ASSIGNMENT.replace("proportional", "all_or_nothing"),
	);
}

#[test]
fn grade_writes_a_record_whose_first_revision_every_consumer_reads() {
	let temp = bench();
	let dir = temp.path();
	succeeded(scriptmark(dir, &GRADE));

	let record = record(dir);
	assert_eq!(record.revisions.len(), 1);
	assert_eq!(grades(dir, None), graded(100.0, 50.0));
	let evidence = &record.evidence;
	assert_eq!(evidence.assignment.name, "sums");
	assert!(evidence.inputs.tests.is_absolute());
	let sources = &evidence.bundle.specs[0].sources;
	assert!(
		sources.keys().any(|p| p.ends_with("teacher/helper.py")),
		"{sources:?}"
	);
	for student in &evidence.students {
		let submission = student.submission.as_ref().expect("fingerprinted");
		assert_eq!(submission.attempt, Some(1));
		assert_eq!(submission.files.len(), 1);
		assert!(submission.files[0].path.is_absolute());
		assert_eq!(submission.files[0].sha256.len(), 64);
	}

	let summary = succeeded(scriptmark(dir, &["summarize", "out/results.json"]));
	assert!(
		text(&summary).contains("(revision 1 of 1)"),
		"{}",
		text(&summary)
	);
	succeeded(scriptmark(
		dir,
		&["export", "out/results.json", "-o", "out/grades.csv"],
	));
	let csv = std::fs::read_to_string(dir.join("out/grades.csv")).unwrap();
	assert!(csv.contains("local:bob,,,graded,,5,10,50,50,"), "{csv}");
	succeeded(scriptmark(
		dir,
		&["report", "out/results.json", "-o", "out/report.html"],
	));
	assert!(dir.join("out/report.html").exists());
}

#[test]
fn rescoring_runs_nothing_and_keeps_the_earlier_revision() {
	let temp = bench();
	let dir = temp.path();
	succeeded(scriptmark(dir, &GRADE));
	let before = record(dir);

	all_or_nothing(dir);
	let output = succeeded(rescore(dir, &[]));
	// The count is coloured; the words around it are not.
	for words in ["revision 2 against revision 1: ", " of 2 students changed"] {
		assert!(text(&output).contains(words), "{}", text(&output));
	}
	let after = record(dir);
	let changes = scriptmark::record::diff(Some(&after.revisions[0]), &after.revisions[1]);
	assert_eq!(
		changes
			.iter()
			.map(|c| c.student_id.as_str())
			.collect::<Vec<_>>(),
		["local:bob"]
	);

	assert_eq!(after.digest, before.digest, "the evidence is untouched");
	assert_eq!(after.revisions[0], before.revisions[0]);
	assert_eq!(grades(dir, Some(1)), graded(100.0, 50.0));
	assert_eq!(grades(dir, None), graded(100.0, 0.0));

	let summary = succeeded(scriptmark(
		dir,
		&["summarize", "out/results.json", "--revision", "1"],
	));
	assert!(
		text(&summary).contains("(revision 1 of 2)"),
		"{}",
		text(&summary)
	);
	succeeded(scriptmark(
		dir,
		&[
			"export",
			"out/results.json",
			"--revision",
			"2",
			"-o",
			"out/grades.csv",
		],
	));
	let csv = std::fs::read_to_string(dir.join("out/grades.csv")).unwrap();
	assert!(csv.contains("local:bob,,,graded,,0,10,0,0,"), "{csv}");

	refused(rescore(dir, &[]), "nothing to rescore: revision 2");
	refused(
		scriptmark(dir, &["summarize", "out/results.json", "--revision", "3"]),
		"there is no revision 3",
	);
}

#[test]
fn rescoring_refuses_evidence_whose_tests_submissions_or_matching_changed() {
	type Change = fn(&Path);
	let changes: [(&str, Change, &str); 9] = [
		(
			"spec",
			|dir| {
				let spec = std::fs::read_to_string(dir.join("tests/test_sum.toml")).unwrap();
				write(
					dir,
					"tests/test_sum.toml",
					&spec.replace("expect = -3", "expect = 3"),
				);
			},
			"test spec 'sum' changed",
		),
		(
			"undeclared helper",
			|dir| {
				write(
					dir,
					"tests/teacher/helper.py",
					"def equal(a, b):\n    return True\n",
				)
			},
			"teacher/helper.py, which test spec 'sum' reads, changed",
		),
		(
			"student file",
			|dir| {
				write(
					dir,
					"submissions/bob_sum.py",
					"def add(a, b):\n    return a + b\n",
				)
			},
			"local:bob's submitted files changed",
		),
		(
			"new student",
			|dir| {
				write(
					dir,
					"submissions/carol_sum.py",
					"def add(a, b):\n    return a + b\n",
				)
			},
			"student local:carol was not graded",
		),
		(
			"removed student",
			|dir| std::fs::remove_file(dir.join("submissions/bob_sum.py")).unwrap(),
			"student local:bob is no longer in the input",
		),
		(
			"matching",
			|dir| {
				let assignment = std::fs::read_to_string(dir.join("assignment.toml")).unwrap();
				write(
					dir,
					"assignment.toml",
					&(assignment
						+ "[[matching.items]]\nid = 'sum'\nfiles = ['sum.py', 'add.py']\n"),
				);
			},
			"the [matching] rules changed",
		),
		(
			"Canvas ids",
			|dir| {
				let assignment = std::fs::read_to_string(dir.join("assignment.toml")).unwrap();
				write(
					dir,
					"assignment.toml",
					&assignment.replace(
						"name = \"sums\"",
						"name = \"sums\"\ncanvas_course_id = 7\ncanvas_assignment_id = 8",
					),
				);
			},
			"the assignment's Canvas ids are course 7 and assignment 8, not course none",
		),
		(
			"attempt policy",
			|dir| {
				let assignment = std::fs::read_to_string(dir.join("assignment.toml")).unwrap();
				write(
					dir,
					"assignment.toml",
					&assignment.replace(
						"name = \"sums\"",
						"name = \"sums\"\nattempt_policy = \"earliest\"",
					),
				);
			},
			"the attempt policy is earliest, not latest",
		),
		(
			"file match",
			|dir| {
				std::fs::rename(
					dir.join("submissions/alice_sum.py"),
					dir.join("submissions/alice_add.py"),
				)
				.unwrap()
			},
			"local:alice's file for 'sum' is matched differently",
		),
	];
	for (what, change, why) in changes {
		let temp = bench();
		let dir = temp.path();
		succeeded(scriptmark(dir, &GRADE));
		let saved = std::fs::read(dir.join("out/results.json")).unwrap();
		all_or_nothing(dir);
		change(dir);
		let output = rescore(dir, &[]);
		assert!(!output.status.success(), "{what}: {}", text(&output));
		assert!(
			text(&output).contains(why),
			"{what}: no '{why}' in\n{}",
			text(&output)
		);
		assert_eq!(
			std::fs::read(dir.join("out/results.json")).unwrap(),
			saved,
			"{what}: a refused rescore writes nothing"
		);
	}
}

#[test]
fn a_run_is_scored_by_its_first_rescore() {
	let temp = bench();
	let dir = temp.path();
	succeeded(scriptmark(
		dir,
		&[
			"run",
			"submissions",
			"-t",
			"tests",
			"-o",
			"out/results.json",
		],
	));
	assert!(record(dir).revisions.is_empty());
	let summary = succeeded(scriptmark(dir, &["summarize", "out/results.json"]));
	assert!(text(&summary).contains("(unscored)"), "{}", text(&summary));
	refused(
		scriptmark(dir, &["export", "out/results.json"]),
		"has no score revision yet",
	);

	let output = succeeded(rescore(dir, &[]));
	assert!(
		text(&output).contains("revision 1 is the first score"),
		"{}",
		text(&output)
	);
	assert_eq!(grades(dir, None), graded(100.0, 50.0));
}

#[test]
fn a_record_with_rescored_revisions_is_replaced_only_by_force() {
	let temp = bench();
	let dir = temp.path();
	succeeded(scriptmark(dir, &GRADE));
	// One revision is the grade's own: grading again replaces it, as it always has.
	succeeded(scriptmark(dir, &GRADE));
	all_or_nothing(dir);
	succeeded(rescore(dir, &[]));
	let saved = std::fs::read(dir.join("out/results.json")).unwrap();

	refused(scriptmark(dir, &GRADE), "holds 2 score revisions");
	refused(
		scriptmark(
			dir,
			&[
				"match",
				"submissions",
				"-t",
				"tests",
				"-o",
				"out/results.json",
			],
		),
		"holds 2 score revisions",
	);
	assert_eq!(std::fs::read(dir.join("out/results.json")).unwrap(), saved);
	succeeded(scriptmark(
		dir,
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"-o",
			"out/again.json",
		],
	));

	// There is no short form to type by accident.
	let mut short = GRADE.to_vec();
	short.push("-f");
	refused(scriptmark(dir, &short), "unexpected argument '-f'");
	assert_eq!(std::fs::read(dir.join("out/results.json")).unwrap(), saved);

	// --force grades afresh over it, says what it discards, and composes with --fresh.
	let mut force = GRADE.to_vec();
	force.extend(["--force", "--fresh"]);
	let output = succeeded(scriptmark(dir, &force));
	assert!(
		text(&output).contains("holds 2 score revisions, which it alone records; --force"),
		"{}",
		text(&output)
	);
	let replaced = record(dir);
	assert_eq!(replaced.revisions.len(), 1);
	assert_eq!(
		grades(dir, None),
		graded(100.0, 0.0),
		"under the current policy"
	);
}

#[test]
fn grades_push_names_its_revision_and_keeps_to_the_records_assignment() {
	let temp = bench();
	let dir = temp.path();
	let with_canvas = ASSIGNMENT.replace(
		"name = \"sums\"",
		"name = \"sums\"\ncanvas_course_id = 7\ncanvas_assignment_id = 8",
	);
	write(dir, "assignment.toml", &with_canvas);
	succeeded(scriptmark(dir, &GRADE));
	write(
		dir,
		"assignment.toml",
		&with_canvas.replace("proportional", "all_or_nothing"),
	);
	succeeded(rescore(dir, &[]));

	// Each refusal comes before Canvas is contacted, or any token is needed.
	let push = |extra: &[&str]| {
		let mut args = vec![
			"grades-push",
			"--canvas-url",
			"http://127.0.0.1:9",
			"--course-id",
			"7",
			"out/results.json",
		];
		args.extend_from_slice(extra);
		Command::new(env!("CARGO_BIN_EXE_scriptmark"))
			.current_dir(dir)
			.env_remove("CANVAS_TOKEN")
			.args(args)
			.output()
			.unwrap()
	};
	refused(
		push(&["--assignment-id", "8"]),
		"name the one to push with --revision",
	);
	refused(
		push(&["--assignment-id", "9", "--revision", "2"]),
		"was graded for Canvas assignment 8, not 9",
	);
	refused(
		push(&["--assignment-id", "8", "--revision", "3"]),
		"there is no revision 3",
	);
}

#[test]
fn database_sessions_are_revisions_of_one_evidence() {
	let temp = bench();
	let dir = temp.path();
	let mut args = GRADE.to_vec();
	args.extend(["--db", "grades.db"]);
	succeeded(scriptmark(dir, &args));
	all_or_nothing(dir);
	succeeded(rescore(dir, &["--db", "grades.db"]));

	let digest = record(dir).digest;
	let db = scriptmark::db::Database::open(&dir.join("grades.db")).unwrap();
	let sessions = db.list_sessions().unwrap();
	assert_eq!(
		sessions
			.iter()
			.map(|s| (s.revision, s.evidence.as_str()))
			.collect::<Vec<_>>(),
		[(2, digest.as_str()), (1, digest.as_str())]
	);
	let bob = |session: i64| {
		db.get_results(session)
			.unwrap()
			.into_iter()
			.find(|r| r.student_id == "local:bob")
			.unwrap()
			.final_grade
	};
	assert_eq!(bob(sessions[0].id), Some(0.0));
	assert_eq!(bob(sessions[1].id), Some(50.0));
	let listed = succeeded(scriptmark(dir, &["db", "sessions", "--db", "grades.db"]));
	assert!(text(&listed).contains(&digest[..12]), "{}", text(&listed));
}

#[test]
fn any_revision_can_be_saved_to_the_database_later_and_only_once() {
	let temp = bench();
	let dir = temp.path();
	let save = |extra: &[&str]| {
		let mut args = vec!["db", "save", "out/results.json", "--db", "grades.db"];
		args.extend_from_slice(extra);
		scriptmark(dir, &args)
	};
	succeeded(scriptmark(
		dir,
		&[
			"run",
			"submissions",
			"-t",
			"tests",
			"-o",
			"out/results.json",
		],
	));
	refused(save(&[]), "has no score revision yet");
	succeeded(rescore(dir, &[]));
	all_or_nothing(dir);
	succeeded(rescore(dir, &[]));

	let first = succeeded(save(&["--revision", "1"]));
	assert!(
		text(&first).contains("session #1, revision 1"),
		"{}",
		text(&first)
	);
	let again = succeeded(save(&["--revision", "1"]));
	assert!(
		text(&again).contains("Revision 1 is already in grades.db as session #1"),
		"{}",
		text(&again)
	);
	succeeded(save(&[]));
	let db = scriptmark::db::Database::open(&dir.join("grades.db")).unwrap();
	assert_eq!(
		db.list_sessions()
			.unwrap()
			.iter()
			.map(|s| s.revision)
			.collect::<Vec<_>>(),
		[2, 1]
	);
}

#[test]
fn results_from_before_grading_records_are_refused_by_every_reader() {
	let temp = bench();
	let dir = temp.path();
	write(dir, "out/results.json", "[]");
	for args in [
		&["summarize", "out/results.json"][..],
		&["export", "out/results.json"],
		&["report", "out/results.json"],
		&["rescore", "out/results.json"],
	] {
		refused(scriptmark(dir, args), "before grading records");
	}
}

#[test]
fn the_assignment_name_is_a_label_so_declaring_items_later_rescores() {
	let temp = bench();
	let dir = temp.path();
	// Graded with items derived from the specs, under the folder's name...
	std::fs::remove_file(dir.join("assignment.toml")).unwrap();
	succeeded(scriptmark(dir, &GRADE));
	let derived = record(dir).evidence.assignment.name;
	assert!(record(dir).revisions[0].policy.derived_items);
	// ...then weighed, as the note `grade` printed suggests, under a name of its own.
	write(
		dir,
		"assignment.toml",
		&ASSIGNMENT
			.replace("name = \"sums\"", "name = \"Homework 3\"")
			.replace("proportional", "all_or_nothing"),
	);
	succeeded(rescore(dir, &[]));
	let record = record(dir);
	assert_eq!(record.evidence.assignment.name, derived);
	assert!(!record.revisions[1].policy.derived_items);
	assert_eq!(grades(dir, None), graded(100.0, 0.0));
}

#[test]
fn a_replaced_archive_is_a_changed_submission_even_behind_a_stale_extraction() {
	let temp = bench();
	let dir = temp.path();
	let zip_of = |content: &str| {
		std::fs::remove_file(dir.join("submissions/bob_sum.py")).ok();
		let file = std::fs::File::create(dir.join("submissions/bob_hw.zip")).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		zip.start_file("sum.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		std::io::Write::write_all(&mut zip, content.as_bytes()).unwrap();
		zip.finish().unwrap();
	};
	zip_of("def add(a, b):\n    return abs(a) + abs(b)\n");
	succeeded(scriptmark(dir, &GRADE));
	let bob = record(dir)
		.evidence
		.students
		.into_iter()
		.find(|r| r.student_id == "local:bob")
		.unwrap();
	assert_eq!(bob.submission.unwrap().archives.len(), 1);

	// The extraction from the first archive stays on disk; the archive itself changed.
	zip_of("def add(a, b):\n    return a + b\n");
	all_or_nothing(dir);
	refused(rescore(dir, &[]), "local:bob's submitted files changed");
}

/// A Canvas bundle as `canvas fetch` leaves one, for course 7, assignment 8. Ada tried
/// twice — wrong, then right — Ben once, wrongly, and Cy, who handed nothing in, is excused.
fn canvas_bundle(dir: &Path) {
	let bundle = dir.join("canvas/hw");
	let mut manifest = serde_json::Map::new();
	for (id, source) in [
		(1001, "def add(a, b):\n    return 0\n"),
		(1002, "def add(a, b):\n    return a + b\n"),
		(1003, "def add(a, b):\n    return abs(a) + abs(b)\n"),
	] {
		let path = bundle.join(format!("attachments/{id}/sum.py"));
		write(
			dir,
			path.strip_prefix(dir).unwrap().to_str().unwrap(),
			source,
		);
		manifest.insert(
			id.to_string(),
			serde_json::json!({ "stored": { "path": path, "size": source.len() } }),
		);
	}
	let attempt = |n: u32, attachment: u64| {
		serde_json::json!({
			"user_id": 11, "attempt": n, "workflow_state": "submitted",
			"submitted_at": format!("2026-10-0{n}T08:00:00Z"), "submission_type": "online_upload",
			"attachments": [{ "id": attachment, "filename": "sum.py" }],
		})
	};
	let payload = serde_json::json!({
		"course_id": 7, "assignment_id": 8, "assignment_name": "sums",
		"users": [
			{ "id": 11, "name": "Ada", "sis_user_id": "2024001" },
			{ "id": 12, "name": "Ben", "sis_user_id": "2024002" },
			{ "id": 13, "name": "Cy", "sis_user_id": "2024003" },
		],
		"submissions": [
			{
				"id": 1, "user_id": 11, "attempt": 2, "workflow_state": "submitted",
				"submitted_at": "2026-10-02T08:00:00Z", "submission_type": "online_upload",
				"attachments": [{ "id": 1002, "filename": "sum.py" }],
				"submission_history": [attempt(1, 1001), attempt(2, 1002)],
			},
			{
				"id": 2, "user_id": 12, "attempt": 1, "workflow_state": "submitted",
				"submitted_at": "2026-10-01T09:00:00Z", "submission_type": "online_upload",
				"attachments": [{ "id": 1003, "filename": "sum.py" }],
			},
			{ "id": 3, "user_id": 13, "workflow_state": "unsubmitted", "excused": true },
		],
	});
	write(
		dir,
		"canvas/hw/canvas-payload.json",
		&serde_json::to_string_pretty(&payload).unwrap(),
	);
	write(
		dir,
		"canvas/hw/attachments.json",
		&serde_json::to_string_pretty(&manifest).unwrap(),
	);
}

#[test]
fn a_canvas_record_rescores_from_its_bundle_and_refuses_another_attempt_or_excusal() {
	let temp = bench();
	let dir = temp.path();
	canvas_bundle(dir);
	let grade = [
		"grade",
		"--canvas",
		"canvas/hw",
		"-t",
		"tests",
		"-o",
		"out/results.json",
	];
	succeeded(scriptmark(dir, &grade));
	let evidence = record(dir).evidence;
	assert_eq!(
		(
			evidence.assignment.canvas_course_id,
			evidence.assignment.canvas_assignment_id
		),
		(Some(7), Some(8))
	);
	let ada = evidence
		.students
		.iter()
		.find(|r| r.canvas_user_id == Some(11))
		.unwrap();
	assert_eq!(ada.submission.as_ref().unwrap().attempt, Some(2));
	assert_eq!(
		grades(dir, None)
			.into_iter()
			.map(|(_, grade)| grade)
			.collect::<Vec<_>>(),
		[Some(100.0), Some(50.0), None],
		"Ada on her second attempt; Cy excused"
	);

	// Rescoring reads the bundle and writes nothing into it.
	std::fs::remove_file(dir.join("canvas/hw/input.json")).unwrap();
	all_or_nothing(dir);
	succeeded(rescore(dir, &[]));
	assert!(!dir.join("canvas/hw/input.json").exists());
	assert_eq!(
		grades(dir, None)
			.into_iter()
			.map(|(_, grade)| grade)
			.collect::<Vec<_>>(),
		[Some(100.0), Some(0.0), None]
	);

	write(
		dir,
		"assignment.toml",
		&ASSIGNMENT.replace(
			"name = \"sums\"",
			"name = \"sums\"\nattempt_policy = \"earliest\"",
		),
	);
	let output = rescore(dir, &[]);
	refused(output, "the attempt policy is earliest, not latest");
	write(dir, "assignment.toml", ASSIGNMENT);

	let payload_path = dir.join("canvas/hw/canvas-payload.json");
	let mut payload: serde_json::Value =
		serde_json::from_str(&std::fs::read_to_string(&payload_path).unwrap()).unwrap();
	payload["submissions"][1]["excused"] = true.into();
	std::fs::write(&payload_path, payload.to_string()).unwrap();
	refused(rescore(dir, &[]), "2024002 is excused now");
}
