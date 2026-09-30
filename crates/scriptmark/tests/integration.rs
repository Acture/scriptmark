//! End to end: specs are loaded and prepared, units run through the Python harness, and
//! the judge's verdicts come back in `StudentReport`s. Grouped by the P-674 acceptance
//! line each test pins.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::grading::{Policy, grade_all};
use scriptmark::models::*;
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::prepare;
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::load_spec_str;

/// A scratch directory holding specs, teacher files and one directory per student.
struct Bench {
	dir: tempfile::TempDir,
}

impl Bench {
	fn new() -> Self {
		Self {
			dir: tempfile::tempdir().unwrap(),
		}
	}

	fn path(&self) -> &Path {
		self.dir.path()
	}

	fn write(&self, name: &str, content: &str) -> PathBuf {
		let path = self.path().join(name);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, content).unwrap();
		path
	}

	fn spec(&self, toml: &str) -> TestSpec {
		load_spec_str(toml, self.path()).unwrap_or_else(|e| panic!("{e}"))
	}

	/// A student whose submission is one file.
	fn student(&self, key: &str, file: &str, code: &str) -> StudentSubmission {
		let path = self.write(&format!("students/{key}/{file}"), code);
		StudentSubmission::from_files(key, &[path])
	}
}

async fn grade_with(
	specs: Vec<TestSpec>,
	students: &[StudentSubmission],
	executor: PythonExecutor,
	timeout: u64,
) -> Vec<StudentReport> {
	let executor = Arc::new(executor);
	let bundles = prepare(specs, executor.clone(), timeout)
		.await
		.unwrap_or_else(|e| panic!("{e}"));
	let options = RunOptions {
		concurrency: Some(4),
		..Default::default()
	};
	run_all(students, bundles.into(), executor, &options).await
}

async fn grade(specs: Vec<TestSpec>, students: &[StudentSubmission]) -> Vec<StudentReport> {
	grade_with(specs, students, PythonExecutor::new(), 5).await
}

/// Why a bundle is refused before any student runs.
async fn refusal(spec: TestSpec) -> String {
	prepare(vec![spec], Arc::new(PythonExecutor::new()), 5)
		.await
		.map(|_| ())
		.unwrap_err()
		.to_string()
}

/// Score reports under the default policy: each spec an item worth 1 point.
fn scored(mut reports: Vec<StudentReport>, specs: &[&TestSpec]) -> Vec<StudentReport> {
	scored_with(&mut reports, specs, GradingConfig::default());
	reports
}

fn scored_with(reports: &mut [StudentReport], specs: &[&TestSpec], config: GradingConfig) {
	let items: Vec<GradingItem> = specs
		.iter()
		.map(|s| GradingItem::new(&s.meta.name))
		.collect();
	let policy = Policy::compile(config, true).unwrap_or_else(|e| panic!("{e}"));
	grade_all(reports, &items, &policy).unwrap_or_else(|e| panic!("{e}"));
}

fn reason(report: &StudentReport) -> Option<Reason> {
	report.grade.as_ref().and_then(|g| g.reason())
}

/// Reports come back as a list in input order, so tests look a student up by the id the
/// model renders rather than indexing a map.
fn by_id<'a>(reports: &'a [StudentReport], student_id: &str) -> &'a StudentReport {
	reports
		.iter()
		.find(|r| r.student_id == student_id)
		.unwrap_or_else(|| panic!("no report for '{student_id}'"))
}

fn case<'a>(report: &'a StudentReport, name: &str) -> &'a CaseResult {
	report
		.test_results
		.iter()
		.flat_map(|t| &t.cases)
		.find(|c| c.case_name == name)
		.unwrap_or_else(|| panic!("no case '{name}' in {report:#?}"))
}

fn verdict(c: &CaseResult) -> (TestStatus, Option<Fault>, Option<Cause>) {
	(c.status, c.fault, c.cause)
}

const LARGER: &str = r#"
[meta]
name = "find_larger_number"
file = "lab5.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "3 < 5"
args = [3, 5]
expect = 5

[[cases]]
name = "equal zero"
args = [0, 0]
expect = 0

[[cases]]
name = "negative"
args = [-3, -2]
expect = -2

[[cases]]
name = "invalid type"
args = ["a", 1]
expect_error = "TypeError"
"#;

const ALICE: &str = r#"
def find_larger_number(a, b):
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        raise TypeError("Arguments must be numbers")
    return max(a, b)
"#;

const BOB: &str = r#"
def find_larger_number(a, b):
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        raise TypeError("Arguments must be numbers")
    return min(a, b)
"#;

// ============================================================================
// A complete teacher bundle runs alone — no generator, reference or seed.
// ============================================================================

#[tokio::test]
async fn test_a_fixed_bundle_grades_without_generator_oracle_or_seed() {
	let bench = Bench::new();
	let students = [
		bench.student("alice", "lab5.py", ALICE),
		bench.student("bob", "lab5.py", BOB),
	];
	let results = grade(vec![bench.spec(LARGER)], &students).await;

	let alice = by_id(&results, "alice");
	assert_eq!(alice.status(), TestStatus::Passed);
	assert_eq!(alice.total_passed(), 4);

	let bob = by_id(&results, "bob");
	assert_eq!(
		bob.total_passed(),
		2,
		"min passes 'equal zero' and the TypeError"
	);
	assert_eq!(
		verdict(case(bob, "3 < 5")),
		(TestStatus::Failed, Some(Fault::Student), Some(Cause::Wrong))
	);
	let input = case(bob, "3 < 5").input.clone().unwrap();
	assert_eq!(input.args, vec![serde_json::json!(3), serde_json::json!(5)]);
	assert!(
		bob.test_results[0]
			.file
			.as_deref()
			.is_some_and(|f| f.ends_with("bob/lab5.py")),
		"the graded file is part of the evidence"
	);
}

/// The seam between the input model and the results: `run_all` is the only place a
/// student's delivery outcome and Canvas id reach `StudentReport`, and it is what makes
/// "a non-submitter is never scored zero" work end to end.
#[tokio::test]
async fn test_a_check_that_ignores_expect_still_has_expect_hold() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "lists"
file = "lab.py"
function = "f"
language = "python"

[[cases]]
name = "sorted"
check = "sorted"
expect = [1, 2, 3]

[[cases]]
name = "length"
check = { rhai = "result != () && result.len() == 3" }
expect = [1, 2, 3]
"#,
	);
	let students = [
		bench.student("alice", "lab.py", "def f():\n    return [1, 2, 3]\n"),
		// Sorted, and three long, but not what expect says.
		bench.student("bob", "lab.py", "def f():\n    return [0, 5, 9]\n"),
	];
	let results = grade(vec![spec], &students).await;
	assert_eq!(by_id(&results, "alice").total_passed(), 2);
	let bob = by_id(&results, "bob");
	for name in ["sorted", "length"] {
		assert_eq!(
			verdict(case(bob, name)),
			(TestStatus::Failed, Some(Fault::Student), Some(Cause::Wrong)),
			"{name}"
		);
	}
}

#[tokio::test]
async fn test_a_concurrency_a_semaphore_cannot_hold_still_runs() {
	let bench = Bench::new();
	let students = [bench.student("alice", "lab5.py", ALICE)];
	let executor = Arc::new(PythonExecutor::new());
	let bundles: Arc<[_]> = prepare(vec![bench.spec(LARGER)], executor.clone(), 5)
		.await
		.unwrap_or_else(|e| panic!("{e}"))
		.into();
	for concurrency in [0, usize::MAX] {
		let options = RunOptions {
			concurrency: Some(concurrency),
			..Default::default()
		};
		let run = run_all(&students, bundles.clone(), executor.clone(), &options);
		let results = tokio::time::timeout(std::time::Duration::from_secs(30), run)
			.await
			.unwrap_or_else(|_| panic!("concurrency {concurrency} hung"));
		assert_eq!(by_id(&results, "alice").total_passed(), 4, "{concurrency}");
	}
}

#[tokio::test]
async fn test_run_all_stamps_identity_and_outcome_onto_every_report() {
	let bench = Bench::new();
	let mut alice = bench.student("alice", "lab5.py", ALICE);
	alice.roster_match = RosterMatch::Matched(0);
	alice.identity.canvas_user_id = Some(101);
	alice.identity.name = Some("Alice".to_string());

	let mut dan_identity = StudentIdentity::number("dan");
	dan_identity.canvas_user_id = Some(105);
	let absent = StudentSubmission::not_submitted(dan_identity, 1, None);

	let spec = bench.spec(LARGER);
	let results = grade(vec![spec.clone()], &[alice, absent]).await;
	assert_eq!(results.len(), 2, "a non-submitter must still get a row");

	let alice = by_id(&results, "alice");
	assert_eq!(alice.submission_state, SubmissionOutcome::Executable);
	assert_eq!(alice.canvas_user_id, Some(101));
	assert_eq!(alice.student_name.as_deref(), Some("Alice"));

	let dan = by_id(&results, "dan");
	assert_eq!(dan.submission_state, SubmissionOutcome::NotSubmitted);
	assert_eq!(dan.canvas_user_id, Some(105));
	assert!(dan.test_results.is_empty());

	// And grading honours it: no grade by default, rather than a zero that would be pushed
	// to Canvas as if the student had earned it.
	let graded = scored(results, &[&spec]);
	assert_eq!(by_id(&graded, "alice").final_grade(), Some(100.0));
	assert_eq!(by_id(&graded, "dan").final_grade(), None);
	assert_eq!(reason(by_id(&graded, "dan")), Some(Reason::NotSubmitted));
}

/// A submitter the roster does not list still has runnable code, and a teacher needs that
/// output to work out why the two disagree.
#[tokio::test]
async fn test_a_submitter_absent_from_the_roster_is_still_executed() {
	let bench = Bench::new();
	let mut stranger = bench.student("alice", "lab5.py", ALICE);
	stranger.roster_match = RosterMatch::NotInRoster;
	assert_eq!(stranger.outcome(), SubmissionOutcome::ReceivedUnmatched);

	let spec = bench.spec(LARGER);
	let results = grade(vec![spec.clone()], &[stranger]).await;
	assert_eq!(results[0].total_passed(), 4, "their tests must still run");
	assert_eq!(
		results[0].submission_state,
		SubmissionOutcome::ReceivedUnmatched
	);
	// Run, reported — but not graded until the identity clash is resolved.
	let graded = scored(results, &[&spec]);
	assert_eq!(graded[0].final_grade(), None);
	assert_eq!(reason(&graded[0]), Some(Reason::PendingReview));
}

#[tokio::test]
async fn test_vars_are_student_globals_and_names_in_scope() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "vars"
file = "lab.py"
language = "python"

[vars]
LIMIT = 10
PAIR = [3, 4]

[[cases]]
name = "global"
function = "limit"
expect = 10

[[cases]]
name = "ref"
function = "total"
args = ["$PAIR"]
expect = 7

[[cases]]
name = "literal dollar"
function = "echo"
args = ["$$5"]
expect = "$5"
"#,
	);
	let alice = bench.student(
		"alice",
		"lab.py",
		"def limit():\n    return LIMIT\n\ndef total(pair):\n    return sum(pair)\n\ndef echo(x):\n    return x\n",
	);
	let results = grade(vec![spec], &[alice]).await;
	assert_eq!(results[0].total_passed(), 3, "{:#?}", results[0]);
}

#[tokio::test]
async fn test_parametrized_cases_with_rhai_and_reference_oracles() {
	let bench = Bench::new();
	bench.write(
		"reference/lab.py",
		"def larger(a, b):\n    return a if a >= b else b\n",
	);
	let spec = bench.spec(
		r#"
[meta]
name = "larger"
file = "lab.py"
function = "larger"
language = "python"

[[cases]]
name = "rhai"
[[cases.parametrize.args]]
a = "int(-100, 100)"
[[cases.parametrize.args]]
b = "int(-100, 100)"
[cases.parametrize.random]
count = 5
seed = 42
[cases.parametrize.oracle]
rhai = "if a >= b { a } else { b }"

[[cases]]
name = "reference"
[cases.parametrize]
args = [{ a = "int(-100, 100)" }, { b = "int(-100, 100)" }]
[cases.parametrize.random]
count = 5
seed = 7
[cases.parametrize.oracle]
reference = "reference/lab.py"
"#,
	);
	let students = [
		bench.student(
			"alice",
			"lab.py",
			"def larger(a, b):\n    return max(a, b)\n",
		),
		bench.student("bob", "lab.py", "def larger(a, b):\n    return min(a, b)\n"),
	];
	let results = grade(vec![spec], &students).await;
	assert_eq!(by_id(&results, "alice").total_passed(), 10);
	assert!(by_id(&results, "bob").total_passed() < 10);
}

/// More random cases in an item never change what the item is worth — the P-677 case: the
/// grade is by item points, not by how many cases each item happens to run.
#[tokio::test]
async fn test_more_generated_cases_do_not_change_an_items_worth() {
	let bench = Bench::new();
	let random = |count: usize| {
		bench.spec(&format!(
			r#"
[meta]
name = "random"
file = "lab.py"
function = "larger"
language = "python"

[[cases]]
name = "random"
[[cases.parametrize.args]]
a = "int(0, 10)"
[[cases.parametrize.args]]
b = "int(20, 30)"
[cases.parametrize.random]
count = {count}
seed = 1
[cases.parametrize.oracle]
rhai = "if a >= b {{ a }} else {{ b }}"
"#
		))
	};
	let fixed = bench.spec(
		r#"
[meta]
name = "fixed"
file = "lab.py"
function = "larger"
language = "python"

[[cases]]
name = "equal"
args = [1, 1]
expect = 1
"#,
	);
	let students = [
		bench.student(
			"alice",
			"lab.py",
			"def larger(a, b):\n    return max(a, b)\n",
		),
		bench.student("bob", "lab.py", "def larger(a, b):\n    return min(a, b)\n"),
	];

	let mut grades = Vec::new();
	for count in [5, 50] {
		let random = random(count);
		let results = grade(vec![random.clone(), fixed.clone()], &students).await;
		assert_eq!(by_id(&results, "bob").total_cases(), count + 1);
		let graded = scored(results, &[&random, &fixed]);
		let grade = |id: &str| {
			let g = by_id(&graded, id).grade.clone().unwrap();
			(g.max, g.final_grade())
		};
		grades.push((grade("alice"), grade("bob")));
	}
	// Bob misses every random case and gets the fixed one: half, at 5 cases or 50.
	assert_eq!(grades[0], ((2.0, Some(100.0)), (2.0, Some(50.0))));
	assert_eq!(grades[0], grades[1]);
}

/// Lint runs on the file of the item that asks for it — not whichever file came first —
/// and says when there was no such file, or when the tool itself failed.
#[tokio::test]
async fn test_lint_runs_on_the_declaring_items_file() {
	let bench = Bench::new();
	// Each item its own file and function, so one file cannot stand in for the other.
	let spec = |name: &str, file: &str, lint: &str| {
		let function = &name[..1];
		bench.spec(&format!(
			"[meta]\nname = \"{name}\"\nfile = \"{file}\"\nfunction = \"{function}\"\nlanguage = \"python\"\n\
			 {lint}\n[[cases]]\nname = \"one\"\nexpect = 1\n"
		))
	};
	// A line per `BAD`: a finding for each.
	let grep = "[lint]\ncommand = \"grep BAD {file}\"\nmax_warnings = 1\n";
	let specs = vec![spec("first", "a.py", ""), spec("second", "b.py", grep)];
	let dirty = "def f():\n    return 1  # BAD\n";
	let clean = "def s():\n    return 1\n";

	let both = StudentSubmission::from_files(
		"alice",
		&[
			bench.write("students/alice/a.py", dirty),
			bench.write("students/alice/b.py", clean),
		],
	);
	let first_only = bench.student("bob", "a.py", dirty);
	let results = grade(specs, &[both, first_only]).await;
	assert_eq!(
		by_id(&results, "alice").lint,
		Some(LintOutcome::Scored { score: 100.0 })
	);
	assert_eq!(by_id(&results, "bob").lint, Some(LintOutcome::NoFile));

	let broken = "[lint]\ncommand = \"sh -c exit${IFS}3\"\n";
	let results = grade(
		vec![spec("second", "b.py", broken)],
		&[bench.student("carol", "b.py", clean)],
	)
	.await;
	assert!(matches!(results[0].lint, Some(LintOutcome::Failed { .. })));
}

/// A teacher module that cannot load is refused before any student runs — never a class
/// of zeros for the teacher's bug. (A teacher fault that only shows up while running is
/// withheld per student; `grading` pins that.)
#[tokio::test]
async fn test_a_teacher_module_that_fails_to_load_grades_nobody() {
	let bench = Bench::new();
	bench.write("helpers/broken.py", "raise RuntimeError('teacher bug')\n");
	let spec = bench.spec(
		r#"
[meta]
name = "larger"
file = "lab.py"
function = "larger"
language = "python"
imports = ["helpers/broken.py"]

[[cases]]
name = "one"
args = [1, 2]
expect = 2
"#,
	);
	assert!(
		refusal(spec)
			.await
			.contains("teacher module failed to import")
	);
}

// ============================================================================
// Changing helper imports does not change isolation.
// ============================================================================

const COUNTER: &str = "calls = 0\n\ndef bump():\n    global calls\n    calls += 1\n    return calls\n\ndef bump_too():\n    return bump()\n";

#[tokio::test]
async fn test_imports_and_function_overrides_do_not_change_isolation() {
	let bench = Bench::new();
	bench.write("helpers/teacher.py", "HELPER = 1\n");
	let plain = r#"
[meta]
name = "plain"
file = "lab.py"
function = "bump"
language = "python"
[[cases]]
name = "first"
expect = 1
[[cases]]
name = "second"
expect = 1
"#;
	let with_import = plain
		.replace("name = \"plain\"", "name = \"imports\"")
		.replace(
			"language = \"python\"",
			"language = \"python\"\nimports = [\"helpers/teacher.py\"]",
		);
	let with_override = plain
		.replace("name = \"plain\"", "name = \"override\"")
		.replace(
			"name = \"second\"",
			"name = \"second\"\nfunction = \"bump_too\"",
		);
	let specs = vec![
		bench.spec(plain),
		bench.spec(&with_import),
		bench.spec(&with_override),
	];
	let results = grade(specs, &[bench.student("alice", "lab.py", COUNTER)]).await;
	for item in &results[0].test_results {
		for c in &item.cases {
			assert_eq!(
				c.status,
				TestStatus::Passed,
				"{}: '{}' saw another case's state: {:?}",
				item.item_id,
				c.case_name,
				c.failure
			);
		}
	}
}

#[tokio::test]
async fn test_a_scenario_shares_state_on_purpose() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "counter"
file = "lab.py"
function = "bump"
language = "python"
[[scenarios]]
name = "counting"
[[scenarios.steps]]
name = "one"
expect = 1
[[scenarios.steps]]
name = "two"
expect = 2
"#,
	);
	let results = grade(vec![spec], &[bench.student("alice", "lab.py", COUNTER)]).await;
	assert_eq!(results[0].total_passed(), 2);
	assert!(
		results[0].test_results[0]
			.cases
			.iter()
			.any(|c| c.case_name == "counting / two")
	);
}

// ============================================================================
// Setup runs exactly once in its declared scope.
// ============================================================================

#[tokio::test]
async fn test_setup_runs_once_per_unit_and_once_per_scenario() {
	let bench = Bench::new();
	let unit_log = bench.path().join("unit.log");
	let scenario_log = bench.path().join("scenario.log");
	let spec = bench.spec(&format!(
		r#"
[meta]
name = "setup_count"
file = "lab.py"
function = "echo"
language = "python"

[vars]
UNIT_LOG = {unit_log:?}
SCENARIO_LOG = {scenario_log:?}

[[setup]]
id = "unit"
function = "record"
args = ["$UNIT_LOG"]

[[cases]]
name = "a"
args = ["$unit"]
expect = "recorded"

[[cases]]
name = "b"
args = ["$unit"]
expect = "recorded"

[[scenarios]]
name = "s"
[[scenarios.setup]]
id = "scenario"
function = "record"
args = ["$SCENARIO_LOG"]
[[scenarios.steps]]
name = "x"
args = ["$scenario"]
expect = "recorded"
[[scenarios.steps]]
name = "y"
args = ["$scenario"]
expect = "recorded"
"#
	));
	let student = bench.student(
		"alice",
		"lab.py",
		"def record(path):\n    with open(path, 'a') as fh:\n        fh.write('ran\\n')\n    return 'recorded'\n\ndef echo(x):\n    return x\n",
	);
	let results = grade(vec![spec], &[student]).await;
	assert_eq!(results[0].total_passed(), 4, "{:#?}", results[0]);

	let lines = |p: &Path| std::fs::read_to_string(p).unwrap().lines().count();
	assert_eq!(
		lines(&unit_log),
		3,
		"top-level setup: once per case, once per scenario"
	);
	assert_eq!(lines(&scenario_log), 1, "scenario setup: once per scenario");
}

// ============================================================================
// Timeouts belong to calls.
// ============================================================================

#[tokio::test]
async fn test_timeouts_belong_to_the_call_that_hung() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "timeouts"
file = "lab.py"
language = "python"

[[cases]]
name = "hangs"
function = "spin"
expect = 1

[[cases]]
name = "slow but allowed"
function = "nap"
timeout = 3
expect = 1

[[cases]]
name = "neighbour"
function = "ok"
expect = 1

[[scenarios]]
name = "s"
[[scenarios.steps]]
name = "hang 1"
function = "spin"
expect = 1
[[scenarios.steps]]
name = "hang 2"
function = "swallow"
expect = 1
[[scenarios.steps]]
name = "hang 3"
function = "spin"
expect = 1
[[scenarios.steps]]
name = "after"
function = "ok"
expect = 1
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"import time\n\ndef spin():\n    while True:\n        pass\n\ndef swallow():\n    try:\n        while True:\n            pass\n    except:\n        return 1\n\ndef nap():\n    time.sleep(1.5)\n    return 1\n\ndef ok():\n    return 1\n",
	);
	let results = grade_with(vec![spec], &[student], PythonExecutor::new(), 1).await;
	let r = &results[0];
	let timeout = (
		TestStatus::Timeout,
		Some(Fault::Student),
		Some(Cause::Timeout),
	);
	assert_eq!(verdict(case(r, "hangs")), timeout);
	assert_eq!(case(r, "slow but allowed").status, TestStatus::Passed);
	assert_eq!(case(r, "neighbour").status, TestStatus::Passed);
	assert_eq!(verdict(case(r, "s / hang 1")), timeout);
	assert_eq!(
		verdict(case(r, "s / hang 2")),
		timeout,
		"a bare except cannot turn a timeout into a pass"
	);
	assert_eq!(verdict(case(r, "s / hang 3")), timeout);
	assert_eq!(
		case(r, "s / after").status,
		TestStatus::Passed,
		"three hangs do not exhaust the CPU limit before the last step"
	);
}

// ============================================================================
// stdout is evidence, never noise on the protocol.
// ============================================================================

#[tokio::test]
async fn test_a_printing_student_passes_and_their_output_is_evidence() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "print"
file = "lab.py"
function = "greet"
language = "python"

[[cases]]
name = "value"
args = ["ada"]
expect = "hi ada"

[[cases]]
name = "output"
args = ["ada"]
expected_stdout = "greeting ada\n"
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"import random, csv, json\nprint('module noise')\n\ndef greet(name):\n    print('greeting', name)\n    return 'hi ' + name\n",
	);
	let results = grade(vec![spec], &[student]).await;
	let value = case(&results[0], "value");
	assert_eq!(value.status, TestStatus::Passed, "{value:?}");
	assert_eq!(value.stdout.as_deref(), Some("greeting ada\n"));
	assert_eq!(case(&results[0], "output").status, TestStatus::Passed);
}

#[tokio::test]
async fn test_script_cases_see_their_stdin() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "io"
file = "io.py"
language = "python"

[[cases]]
name = "sum"
script = true
stdin = "3\n1 2 3\n"
expected_stdout = "n? 6\n"

[[cases]]
name = "tolerant"
script = true
stdin = "1\n5\n"
expected_stdout = "n? 5"
check = "text"
"#,
	);
	let student = bench.student(
		"alice",
		"io.py",
		"import sys\nn = int(input('n? '))\nprint(sum(int(x) for x in sys.stdin.read().split()))\n",
	);
	let results = grade(vec![spec], &[student]).await;
	assert_eq!(results[0].total_passed(), 2, "{:#?}", results[0]);
}

// ============================================================================
// Every failure has an owner.
// ============================================================================

#[tokio::test]
async fn test_every_failure_has_an_owner() {
	let bench = Bench::new();
	bench.write(
		"helpers/teacher.py",
		"def crashes(result, expected):\n    return result.nope\n\ndef rejects(result, expected):\n    assert isinstance(result, list), 'expected a list'\n    return True\n\ndef make():\n    raise RuntimeError('teacher bug')\n",
	);
	let spec = bench.spec(
		r#"
[meta]
name = "owners"
file = "lab.py"
language = "python"
imports = ["helpers/teacher.py"]

[[cases]]
name = "missing function"
function = "nope_not_here_at_all"
expect = 1

[[cases]]
name = "raises"
function = "boom"
expect = 1

[[cases]]
name = "wrong"
function = "five"
expect = 6

[[cases]]
name = "checker crashes"
function = "five"
check = { function = "crashes" }

[[cases]]
name = "checker rejects"
function = "five"
check = { function = "rejects" }

[[cases]]
name = "rhai cannot decide"
function = "five"
check = { rhai = "result != () && result.len() > 0" }

[[cases]]
name = "unserialisable"
function = "clash"
expect = 1

[[scenarios]]
name = "chain"
[[scenarios.steps]]
name = "producer fails"
id = "made"
function = "boom"
expect = 1
[[scenarios.steps]]
name = "consumer"
function = "echo"
args = ["$made"]
expect = 1

[[scenarios]]
name = "teacher setup"
[[scenarios.setup]]
id = "t"
teacher = "make"
[[scenarios.steps]]
name = "never runs"
function = "five"
expect = 5
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"def boom():\n    raise KeyError('k')\n\ndef five():\n    return 5\n\ndef clash():\n    return {1: 'a', '1': 'b'}\n\ndef echo(x):\n    return x\n",
	);
	let results = grade(vec![spec], &[student]).await;
	let r = &results[0];
	use Cause::*;
	use Fault::*;
	use TestStatus as S;
	let expected = [
		(
			"missing function",
			(S::Missing, Some(Student), Some(NoTarget)),
		),
		("raises", (S::Error, Some(Student), Some(Raised))),
		("wrong", (S::Failed, Some(Student), Some(Wrong))),
		("checker crashes", (S::Failed, Some(Student), Some(Checker))),
		(
			"checker rejects",
			(S::Failed, Some(Student), Some(Rejected)),
		),
		(
			"rhai cannot decide",
			(S::Failed, Some(Student), Some(Checker)),
		),
		(
			"unserialisable",
			(S::Error, Some(Student), Some(Unserialisable)),
		),
		(
			"chain / producer fails",
			(S::Error, Some(Student), Some(Raised)),
		),
		(
			"chain / consumer",
			(S::Error, Some(Student), Some(Dependency)),
		),
		(
			"teacher setup / never runs",
			(S::Error, Some(Teacher), Some(Setup)),
		),
	];
	for (name, want) in expected {
		assert_eq!(
			verdict(case(r, name)),
			want,
			"{name}: {:?}",
			case(r, name).failure
		);
	}
}

#[tokio::test]
async fn test_failures_before_any_call_have_owners_too() {
	let bench = Bench::new();
	let spec = bench.spec(LARGER);
	let students = [
		bench.student("nofile", "other.txt", "hello"),
		bench.student(
			"syntax",
			"lab5.py",
			"def find_larger_number(a, b)\n    return a\n",
		),
		bench.student("exits", "lab5.py", "exit()\n"),
	];
	let results = grade(vec![spec.clone()], &students).await;
	let first = |id: &str| verdict(&by_id(&results, id).test_results[0].cases[0]);
	assert_eq!(
		first("nofile"),
		(
			TestStatus::Missing,
			Some(Fault::Student),
			Some(Cause::NoFile)
		)
	);
	assert_eq!(
		first("syntax"),
		(TestStatus::Error, Some(Fault::Student), Some(Cause::Syntax))
	);
	assert_eq!(
		first("exits"),
		(TestStatus::Error, Some(Fault::Student), Some(Cause::Load))
	);

	// A missing file withholds that student by default; syntax and load errors are the
	// student's and score 0.
	let graded = scored(results, &[&spec]);
	assert_eq!(reason(by_id(&graded, "nofile")), Some(Reason::MissingFile));
	assert_eq!(by_id(&graded, "syntax").final_grade(), Some(0.0));
	assert_eq!(by_id(&graded, "exits").final_grade(), Some(0.0));

	// A grader that cannot start Python blames nobody's code.
	let results = grade_with(
		vec![spec.clone()],
		&[bench.student("alice", "lab5.py", ALICE)],
		PythonExecutor::with_python_cmd("/nonexistent/python3"),
		5,
	)
	.await;
	assert_eq!(
		verdict(&results[0].test_results[0].cases[0]),
		(
			TestStatus::Error,
			Some(Fault::Environment),
			Some(Cause::Spawn)
		)
	);
	// And it is no grade at all, never a zero.
	let graded = scored(results, &[&spec]);
	assert_eq!(graded[0].final_grade(), None);
	assert_eq!(reason(&graded[0]), Some(Reason::EnvironmentFault));
}

// ============================================================================
// Illegal or unsupported configuration is refused before any student runs.
// ============================================================================

#[tokio::test]
async fn test_a_bundle_that_cannot_be_honoured_is_refused_before_grading() {
	let bench = Bench::new();
	bench.write(
		"helpers/broken.py",
		"raise RuntimeError('helper is broken')\n",
	);
	bench.write(
		"helpers/decorated.py",
		"@checker('f')\ndef check_f(result, expected):\n    return True\n",
	);
	bench.write("helpers/ok.py", "def two(result):\n    return True\n");
	bench.write(
		"reference/bad.py",
		"def f(x):\n    raise ValueError('no')\n",
	);

	let spec = |body: &str| {
		bench.spec(&format!(
			"[meta]\nname = \"t\"\nfile = \"lab.py\"\nfunction = \"f\"\nlanguage = \"python\"\n{body}"
		))
	};
	let refusals = [
		(
			spec("[[cases]]\nname = \"x\"\nargs = [\"$ghost\"]\nexpect = 1\n"),
			"'$ghost' names nothing in scope",
		),
		(
			spec("[[cases]]\nname = \"x\"\nexpect = 1\n").tap_imports(&bench, "helpers/broken.py"),
			"teacher module failed to import",
		),
		(
			spec("[[cases]]\nname = \"x\"\nexpect = 1\n")
				.tap_imports(&bench, "helpers/decorated.py"),
			"check = { function = \"<checker name>\" }",
		),
		(
			spec("[[cases]]\nname = \"x\"\ncheck = { rhai = \"result.len() > 0\" }\n"),
			"cannot judge a student who returns None",
		),
		(
			spec("[[cases]]\nname = \"x\"\ncheck = { function = \"nowhere\" }\n")
				.tap_imports(&bench, "helpers/ok.py"),
			"checker 'nowhere' is not a function the teacher modules export",
		),
		(
			spec("[[cases]]\nname = \"x\"\ncheck = { function = \"two\" }\n")
				.tap_imports(&bench, "helpers/ok.py"),
			"must take (result, expected, ...)",
		),
		(
			spec(
				"[[cases]]\nname = \"x\"\n[[cases.parametrize.args]]\nx = \"int(0, 1)\"\n[cases.parametrize.random]\ncount = 1\n[cases.parametrize.oracle]\nreference = \"reference/bad.py\"\n",
			),
			"reference implementation 'f' did not return a value",
		),
	];
	for (spec, needle) in refusals {
		let message = refusal(spec).await;
		assert!(
			message.contains(needle),
			"expected {needle:?} in:\n{message}"
		);
	}
}

#[tokio::test]
async fn test_every_name_must_mean_exactly_one_thing() {
	let bench = Bench::new();
	bench.write(
		"helpers/t.py",
		"LIMIT = 3\n\ndef make():\n    return 1\n\ndef chk(result, expected, ghost):\n    return True\n\ndef lenient(result, expected, tol=0.5, **rest):\n    return True\n",
	);
	bench.write("helpers/u.py", "def make():\n    return 2\n");
	bench.write("helpers/out.py", "stdout = 'mine'\n");
	bench.write("reference/none.py", "def f(x):\n    print(x)\n");
	let spec = |body: &str, imports: &[&str]| {
		let imports: Vec<String> = imports.iter().map(|i| format!("{i:?}")).collect();
		bench.spec(&format!(
			"[meta]\nname = \"t\"\nfile = \"lab.py\"\nfunction = \"f\"\nlanguage = \"python\"\nimports = [{}]\n{body}",
			imports.join(", ")
		))
	};
	let t = "helpers/t.py";
	let refusals = [
		(
			spec(
				"[vars]\nLIMIT = 4\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				&[t],
			),
			"'LIMIT' is both a [vars] entry and a teacher export",
		),
		(
			spec(
				"[[setup]]\nid = \"make\"\nfunction = \"f\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				&[t],
			),
			"id 'make' is also a teacher export",
		),
		(
			spec(
				"[[setup]]\nid = \"d\"\nteacher = \"absent\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				&[t],
			),
			"teacher function 'absent' is not exported",
		),
		(
			spec(
				"[[cases]]\nname = \"x\"\ncheck = { function = \"chk\" }\n",
				&[t],
			),
			"asks for 'ghost', which names nothing in scope",
		),
		(
			spec(
				"[[cases]]\nname = \"x\"\nexpect = 1\n",
				&[t, "helpers/u.py"],
			),
			"'make' is exported by more than one teacher module",
		),
		(
			spec("[[cases]]\nname = \"x\"\nexpect = 1\n", &["helpers/out.py"]),
			"exports 'stdout'",
		),
		(
			spec(
				"[[cases]]\nname = \"x\"\n[[cases.parametrize.args]]\nx = \"int(0, 1)\"\n[cases.parametrize.random]\ncount = 1\n[cases.parametrize.oracle]\nreference = \"reference/none.py\"\n",
				&[],
			),
			"returned None",
		),
	];
	for (spec, needle) in refusals {
		let message = refusal(spec).await;
		assert!(
			message.contains(needle),
			"expected {needle:?} in:\n{message}"
		);
	}

	// Defaulted and variadic parameters are filled only when named: this one prepares.
	assert!(
		prepare(
			vec![spec(
				"[[cases]]\nname = \"x\"\ncheck = { function = \"lenient\" }\n",
				&[t]
			)],
			Arc::new(PythonExecutor::new()),
			5
		)
		.await
		.is_ok()
	);

	let nothing = prepare(vec![], Arc::new(PythonExecutor::new()), 5).await;
	assert!(
		nothing
			.map(|_| ())
			.unwrap_err()
			.to_string()
			.contains("no test specs")
	);
	let zero = prepare(vec![bench.spec(LARGER)], Arc::new(PythonExecutor::new()), 0).await;
	assert!(
		zero.map(|_| ())
			.unwrap_err()
			.to_string()
			.contains("between 1 and")
	);
}

#[tokio::test]
async fn test_concurrent_units_write_the_same_file_without_meeting() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "writes"
file = "lab.py"
function = "save"
language = "python"

[[cases]]
name = "one"
args = ["first"]
expect_files = { "out.txt" = "first" }

[[cases]]
name = "two"
args = ["second"]
expect_files = { "out.txt" = "second" }

[[cases]]
name = "three"
args = ["third"]
expect_files = { "out.txt" = "third" }
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"import time\n\ndef save(text):\n    with open('out.txt', 'w') as fh:\n        fh.write(text)\n    time.sleep(0.3)\n",
	);
	let cwd_before: Vec<_> = std::fs::read_dir(".")
		.unwrap()
		.map(|e| e.unwrap().file_name())
		.collect();
	let results = grade(vec![spec], &[student]).await;
	assert_eq!(results[0].total_passed(), 3, "{:#?}", results[0]);
	let cwd_after: Vec<_> = std::fs::read_dir(".")
		.unwrap()
		.map(|e| e.unwrap().file_name())
		.collect();
	assert_eq!(
		cwd_before.len(),
		cwd_after.len(),
		"nothing written to the grader's cwd"
	);
	let submission = bench.path().join("students/alice");
	assert_eq!(
		std::fs::read_dir(submission).unwrap().count(),
		1,
		"nor beside the submission"
	);
}

#[tokio::test]
async fn test_a_failing_student_setup_fails_every_case_it_runs_in() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "loader"
file = "lab.py"
function = "echo"
language = "python"

[[setup]]
id = "data"
function = "load"

[[cases]]
name = "uses it"
args = ["$data"]
expect = 1

[[cases]]
name = "does not"
args = [2]
expect = 2
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"def load():\n    raise FileNotFoundError('data.csv')\n\ndef echo(x):\n    return x\n",
	);
	let results = grade(vec![spec], &[student]).await;
	for name in ["uses it", "does not"] {
		let c = case(&results[0], name);
		assert_eq!(
			verdict(c),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Setup))
		);
		assert!(c.failure.as_ref().unwrap().message.contains("setup 'data'"));
	}
}

#[tokio::test]
async fn test_a_scenario_killed_at_its_deadline_blames_the_call_that_hung() {
	let bench = Bench::new();
	let spec = bench.spec(
		r#"
[meta]
name = "stubborn"
file = "lab.py"
language = "python"

[[scenarios]]
name = "s"
[[scenarios.steps]]
name = "first"
function = "ok"
expect = 1
[[scenarios.steps]]
name = "stubborn"
function = "forever"
expect = 1
[[scenarios.steps]]
name = "after"
function = "ok"
expect = 1
"#,
	);
	let student = bench.student(
		"alice",
		"lab.py",
		"def ok():\n    return 1\n\ndef forever():\n    while True:\n        try:\n            while True:\n                pass\n        except BaseException:\n            pass\n",
	);
	let results = grade_with(vec![spec], &[student], PythonExecutor::new(), 1).await;
	let r = &results[0];
	assert_eq!(case(r, "s / first").status, TestStatus::Passed);
	assert_eq!(
		verdict(case(r, "s / stubborn")),
		(
			TestStatus::Timeout,
			Some(Fault::Student),
			Some(Cause::Killed)
		)
	);
	assert_eq!(
		verdict(case(r, "s / after")),
		(TestStatus::Error, Some(Fault::Student), Some(Cause::NotRun))
	);
}

trait TapImports {
	fn tap_imports(self, bench: &Bench, path: &str) -> Self;
}

impl TapImports for TestSpec {
	/// Add a teacher module after loading, the way a hand-built spec would.
	fn tap_imports(mut self, bench: &Bench, path: &str) -> Self {
		self.meta
			.imports
			.push(bench.path().join(path).to_string_lossy().into_owned());
		self
	}
}
