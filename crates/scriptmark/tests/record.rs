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
	let changes: [(&str, Change, &str); 7] = [
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
			"assignment",
			|dir| {
				let assignment = std::fs::read_to_string(dir.join("assignment.toml")).unwrap();
				write(
					dir,
					"assignment.toml",
					&assignment.replace("name = \"sums\"", "name = \"sums, again\""),
				);
			},
			"the assignment is 'sums, again', not 'sums'",
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
fn a_record_with_rescored_revisions_is_never_replaced() {
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
