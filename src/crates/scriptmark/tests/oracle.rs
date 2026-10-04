//! Reference answers are prepared once, persisted, and replayed without executing them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::models::{StudentReport, StudentSubmission, TestSpec, TestStatus};
use scriptmark::runner::frozen::{Frozen, Generation};
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::{Bundle, prepare};
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::load_spec_str;
use serde_json::{Value, json};

struct Bench(tempfile::TempDir);

impl Bench {
	fn new() -> Self {
		Self(tempfile::tempdir().unwrap())
	}
	fn path(&self) -> &Path {
		self.0.path()
	}
	fn write(&self, name: &str, content: &str) -> PathBuf {
		let path: PathBuf = self.path().join(name);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, content).unwrap();
		path
	}
	fn spec(&self, body: &str) -> TestSpec {
		load_spec_str(
			&format!(
				"[meta]\nname = 't'\nfile = 'lab.py'\nfunction = 'student'\nlanguage = 'python'\n{body}"
			),
			self.path(),
		)
		.unwrap()
	}
	fn student(&self, id: &str, source: &str) -> StudentSubmission {
		StudentSubmission::from_files(id, &[self.write(&format!("students/{id}/lab.py"), source)])
	}
}

async fn prepared(spec: TestSpec, generation: &Generation) -> Result<Vec<Bundle>, String> {
	prepare(vec![spec], generation, Arc::new(PythonExecutor::new()), 3)
		.await
		.map_err(|e| e.to_string())
}

async fn run(bundles: Vec<Bundle>, students: &[StudentSubmission]) -> Vec<StudentReport> {
	run_all(
		students,
		bundles.into(),
		Arc::new(PythonExecutor::new()),
		&RunOptions::default(),
	)
	.await
}

fn all_passed(report: &StudentReport) -> bool {
	report
		.test_results
		.iter()
		.flat_map(|t| &t.cases)
		.all(|c| c.status == TestStatus::Passed)
}

const FIXED: &str = "[[cases]]\nname = 'fixed'\nargs = [4]\n[cases.oracle]\nreference = 'ref.py'\nfunction = 'answer'\n";

#[tokio::test]
async fn fixed_samples_and_seeded_inputs_share_answers_across_students_and_batches() {
	let bench: Bench = Bench::new();
	let counter: PathBuf = bench.path().join("calls.txt");
	bench.write(
		"ref.py",
		&format!(
			"def answer(x):\n    with open({:?}, 'a') as f:\n        f.write('call\\n')\n    return x * 2\n",
			counter
		),
	);
	let body: String = format!(
		"{FIXED}\n[[cases]]\nname = 'generated'\n[cases.parametrize]\nargs = {{ x = 'int(0, 10)' }}\nsamples = [[2], [3]]\n[cases.parametrize.random]\ncount = 4\nseed = 42\n[cases.oracle]\nreference = 'ref.py'\nfunction = 'answer'\n"
	);
	let bundles: Vec<Bundle> = prepared(bench.spec(&body), &Generation::fresh())
		.await
		.unwrap();
	let frozen: Frozen = Frozen::of(&bundles);
	let path: PathBuf = bench.path().join("batch.cases.json");
	frozen.write(&path).unwrap();
	assert_eq!(
		std::fs::read_to_string(&counter).unwrap().lines().count(),
		7
	);
	let students: Vec<StudentSubmission> = vec![
		bench.student("alice", "def student(x):\n    return x * 2\n"),
		bench.student("bob", "def student(x):\n    return x * 2 + 1\n"),
	];
	let reports: Vec<StudentReport> = run(bundles, &students).await;
	assert!(all_passed(&reports[0]));
	assert!(!all_passed(&reports[1]));
	let replay: Generation = Generation::Replay(Frozen::load(&path).unwrap());
	let again: Vec<Bundle> = prepared(bench.spec(&body), &replay).await.unwrap();
	assert_eq!(Frozen::of(&again), frozen);
	let late: Vec<StudentReport> = run(again, &students[..1]).await;
	assert!(all_passed(&late[0]));
	assert_eq!(
		std::fs::read_to_string(counter).unwrap().lines().count(),
		7,
		"neither students nor replay recompute reference answers"
	);
}

#[tokio::test]
async fn stdout_files_stdin_and_vars_use_the_same_observation_contract() {
	let bench: Bench = Bench::new();
	let code: &str = "def answer(x):\n    text = input() + str(x)\n    print(text)\n    with open('out.txt', 'w', encoding='utf-8') as f:\n        f.write(text)\n";
	bench.write("ref.py", code);
	let body: &str = "[vars]\nx = 4\n[[cases]]\nname = 'io'\nargs = ['$x']\nstdin = '你好'\n[cases.oracle]\nreference = 'ref.py'\nfunction = 'answer'\nreturns = false\nstdout = true\nfiles = ['out.txt']\n";
	let bundles: Vec<Bundle> = prepared(bench.spec(body), &Generation::fresh())
		.await
		.unwrap();
	assert_eq!(bundles[0].spec.cases[0].expect, None);
	assert_eq!(
		bundles[0].spec.cases[0].expected_stdout.as_deref(),
		Some("你好4\n")
	);
	let frozen: Frozen = Frozen::from_json(&Frozen::of(&bundles).to_json()).unwrap();
	let students: Vec<StudentSubmission> = vec![
		bench.student("alice", &code.replace("answer", "student")),
		bench.student(
			"bob",
			&code
				.replace("answer", "student")
				.replace("f.write(text)", "f.write('wrong')"),
		),
		bench.student(
			"carol",
			&code
				.replace("answer", "student")
				.replace("print(text)", "print('wrong')"),
		),
	];
	let replayed: Vec<Bundle> = prepared(bench.spec(body), &Generation::Replay(frozen))
		.await
		.unwrap();
	let reports: Vec<StudentReport> = run(replayed, &students).await;
	assert!(all_passed(&reports[0]));
	assert!(!all_passed(&reports[1]));
	assert!(!all_passed(&reports[2]));
}

#[tokio::test]
async fn declared_exception_is_an_answer_but_unexpected_exception_is_a_teacher_failure() {
	let bench: Bench = Bench::new();
	bench.write(
		"ref.py",
		"def answer(x):\n    print('invalid')\n    raise ValueError('negative')\n",
	);
	let body: String = format!("{FIXED}raises = 'ValueError'\nstdout = true\n");
	let bundles: Vec<Bundle> = prepared(bench.spec(&body), &Generation::fresh())
		.await
		.unwrap();
	let frozen: Frozen = Frozen::from_json(&Frozen::of(&bundles).to_json()).unwrap();
	let replayed: Vec<Bundle> = prepared(bench.spec(&body), &Generation::Replay(frozen))
		.await
		.unwrap();
	let reports: Vec<StudentReport> = run(
		replayed,
		&[
			bench.student(
				"alice",
				"def student(x):\n    print('invalid')\n    raise ValueError()\n",
			),
			bench.student(
				"bob",
				"def student(x):\n    print('invalid')\n    return 0\n",
			),
		],
	)
	.await;
	assert!(all_passed(&reports[0]));
	assert!(!all_passed(&reports[1]));
	for source in [
		"def answer(x):\n    raise RuntimeError('broken')\n",
		"def answer(x):\n    return 0\n",
	] {
		bench.write("ref.py", source);
		let error: String = prepared(bench.spec(&body), &Generation::fresh())
			.await
			.unwrap_err();
		assert!(error.contains("unexpected outcome"), "{error}");
	}
}

#[tokio::test]
async fn explicit_none_survives_freezing_as_a_return_expectation() {
	let bench: Bench = Bench::new();
	bench.write("ref.py", "def answer(x):\n    return None\n");
	let body: String = format!("{FIXED}returns = true\n");
	let bundles: Vec<Bundle> = prepared(bench.spec(&body), &Generation::fresh())
		.await
		.unwrap();
	let frozen: Frozen = Frozen::from_json(&Frozen::of(&bundles).to_json()).unwrap();
	let bundles: Vec<Bundle> = prepared(bench.spec(&body), &Generation::Replay(frozen))
		.await
		.unwrap();
	assert_eq!(bundles[0].spec.cases[0].expect, Some(Value::Null));
	let reports: Vec<StudentReport> = run(
		bundles,
		&[
			bench.student("alice", "def student(x):\n    pass\n"),
			bench.student("bob", "def student(x):\n    return 1\n"),
		],
	)
	.await;
	assert!(all_passed(&reports[0]));
	assert!(!all_passed(&reports[1]));
}

#[tokio::test]
async fn changed_sources_configuration_and_damaged_answers_require_fresh_preparation() {
	let bench: Bench = Bench::new();
	let code: &str = "def answer(x):\n    return x * 2\n";
	bench.write("ref.py", code);
	let frozen: Frozen = Frozen::of(
		&prepared(bench.spec(FIXED), &Generation::fresh())
			.await
			.unwrap(),
	);
	for body in [
		FIXED.replace("[4]", "[5]"),
		format!("{FIXED}version = '2'\n"),
	] {
		let error: String = prepared(bench.spec(&body), &Generation::Replay(frozen.clone()))
			.await
			.unwrap_err();
		assert!(error.contains("frozen answers no longer match"), "{error}");
	}
	bench.write("ref.py", "def answer(x):\n    return x * 3\n");
	let error: String = prepared(bench.spec(FIXED), &Generation::Replay(frozen.clone()))
		.await
		.unwrap_err();
	assert!(error.contains("frozen answers no longer match"), "{error}");
	let fresh: Vec<Bundle> = prepared(bench.spec(FIXED), &Generation::fresh())
		.await
		.unwrap();
	assert_eq!(fresh[0].spec.cases[0].expect, Some(json!(12)));
	bench.write("ref.py", code);
	let mut damaged: Value = serde_json::from_str(&frozen.to_json()).unwrap();
	damaged["answers"]["t"]["cases"]["fixed"]["outcome"] = json!({"returned": 123});
	let error: String = prepared(
		bench.spec(FIXED),
		&Generation::Replay(Frozen::from_json(&damaged.to_string()).unwrap()),
	)
	.await
	.unwrap_err();
	assert!(error.contains("checksum"), "{error}");
	let mut missing: Frozen = frozen;
	missing.answers.clear();
	let error: String = prepared(bench.spec(FIXED), &Generation::Replay(missing))
		.await
		.unwrap_err();
	assert!(error.contains("frozen answers are missing"), "{error}");
}

#[tokio::test]
async fn reference_signature_is_checked_for_fixed_and_generated_arguments() {
	let bench: Bench = Bench::new();
	bench.write("ref.py", "def answer(low, high):\n    return low - high\n");
	let error: String = prepared(bench.spec(FIXED), &Generation::fresh())
		.await
		.unwrap_err();
	assert!(error.contains("signature cannot accept"), "{error}");
	let body: &str = "[[cases]]\nname = 'wrong order'\n[cases.parametrize]\nargs = { high = 'int(2, 3)', low = 'int(0, 1)' }\nsamples = [[3, 1]]\n[cases.oracle]\nreference = 'ref.py'\nfunction = 'answer'\n";
	let error: String = prepared(bench.spec(body), &Generation::fresh())
		.await
		.unwrap_err();
	assert!(
		error.contains("do not match") && error.contains("call order"),
		"{error}"
	);
	let error: String = prepared(
		bench.spec(&FIXED.replace("'answer'", "'answr'")),
		&Generation::fresh(),
	)
	.await
	.unwrap_err();
	assert!(
		error.contains("no inspectable public function 'answr'"),
		"{error}"
	);
}

#[tokio::test]
async fn declared_data_and_helpers_are_part_of_the_answer_sources() {
	let bench: Bench = Bench::new();
	bench.write(
		"ref.py",
		"def answer(x):\n    with open('data/value.txt') as f:\n        return x + int(f.read())\n",
	);
	bench.write("data/value.txt", "3");
	bench.write("helper.py", "VALUE = 1\n");
	let body: String = format!("data_files = ['data']\nimports = ['helper.py']\n{FIXED}");
	let frozen: Frozen = Frozen::of(
		&prepared(bench.spec(&body), &Generation::fresh())
			.await
			.unwrap(),
	);
	for (path, changed, original) in [
		("data/value.txt", "4", "3"),
		("helper.py", "VALUE = 2\n", "VALUE = 1\n"),
	] {
		bench.write(path, changed);
		let error: String = prepared(bench.spec(&body), &Generation::Replay(frozen.clone()))
			.await
			.unwrap_err();
		assert!(error.contains("frozen answers no longer match"), "{error}");
		bench.write(path, original);
	}
}

#[tokio::test]
async fn sources_cannot_change_while_answers_are_being_prepared() {
	let bench: Bench = Bench::new();
	let source: PathBuf = bench.path().join("ref.py");
	bench.write("ref.py", &format!("def answer(x):\n    with open({source:?}, 'a') as f:\n        f.write('\\n# changed\\n')\n    return x\n"));
	let error: String = prepared(bench.spec(FIXED), &Generation::fresh())
		.await
		.unwrap_err();
	assert!(
		error.contains("sources changed during preparation"),
		"{error}"
	);
}

#[tokio::test]
async fn a_fixed_rhai_oracle_uses_the_literal_argument_list_and_freezes_it() {
	let bench: Bench = Bench::new();
	let body: &str = "[[cases]]\nname = 'rhai'\nargs = [4]\n[cases.oracle]\nrhai = 'args[0] * 2'\n";
	let bundles: Vec<Bundle> = prepared(bench.spec(body), &Generation::fresh())
		.await
		.unwrap();
	assert_eq!(bundles[0].spec.cases[0].expect, Some(json!(8)));
	let frozen: Frozen = Frozen::from_json(&Frozen::of(&bundles).to_json()).unwrap();
	assert!(!frozen.is_empty());
	assert!(
		prepared(bench.spec(body), &Generation::Replay(frozen))
			.await
			.is_ok()
	);
}

#[tokio::test]
async fn broken_reference_observations_never_become_answers() {
	let bench: Bench = Bench::new();
	let failures: [(&str, &str, &str); 5] = [
		("def answer(x):\n    while True: pass\n", "", "Timeout"),
		(
			"def answer(x):\n    print('a' * 70000)\n",
			"returns = false\nstdout = true\n",
			"stdout was truncated",
		),
		(
			"def answer(x):\n    pass\n",
			"returns = false\nfiles = ['out.txt']\n",
			"readable text file",
		),
		(
			"def answer(x):\n    with open('out.txt', 'w') as f: f.write('x' * (1024 * 1024 + 1))\n",
			"returns = false\nfiles = ['out.txt']\n",
			"readable text file",
		),
		(
			"def answer(x):\n    with open('out.txt', 'wb') as f: f.write(bytes([255]))\n",
			"returns = false\nfiles = ['out.txt']\n",
			"readable text file",
		),
	];
	for (code, settings, needle) in failures {
		bench.write("ref.py", code);
		let body: String = format!(
			"{}{settings}",
			FIXED.replace("args = [4]", "args = [4]\ntimeout = 1")
		);
		let error: String = prepared(bench.spec(&body), &Generation::fresh())
			.await
			.unwrap_err();
		assert!(error.contains(needle), "expected {needle}: {error}");
	}
}

#[test]
fn ambiguous_or_unsupported_sources_are_refused_before_preparation() {
	let bench: Bench = Bench::new();
	bench.write("ref.py", "def answer(x):\n    return x\n");
	let invalid: Vec<String> = vec![
		FIXED.replace("args = [4]", "args = [4]\nexpect = 4"),
		FIXED.replace("function = 'answer'", ""),
		format!("{FIXED}rhai = '1'\n"),
		format!("{FIXED}returns = false\n"),
		format!("{FIXED}raises = 'ValueError'\nreturns = true\n"),
		format!("{FIXED}files = ['../out.txt']\n"),
		format!("{FIXED}files = ['out.txt', 'out.txt']\n"),
		format!("{FIXED}files = ['out.txt', './out.txt']\n"),
		format!("{FIXED}files = ['.']\n"),
		format!(
			"{}stdout = true\n",
			FIXED.replace("args = [4]", "args = [4]\nexpected_stdout = 'x'")
		),
		format!(
			"{}files = ['out.txt']\n",
			FIXED
				.replace(
					"args = [4]",
					"args = [4]\nexpect_files = {{ 'out.txt' = 'x' }}"
				)
				.replace("{{", "{")
				.replace("}}", "}")
		),
		FIXED.replace("'ref.py'", "'missing.py'"),
		format!(
			"{FIXED}[cases.parametrize]\nargs = {{ x = 'int(0, 1)' }}\nsamples = [[1]]\n[cases.parametrize.oracle]\nrhai = 'x'\n"
		),
	];
	for body in invalid {
		let text: String = format!(
			"[meta]\nname = 't'\nfile = 'lab.py'\nfunction = 'student'\nlanguage = 'python'\n{body}"
		);
		assert!(
			load_spec_str(&text, bench.path()).is_err(),
			"accepted {body}"
		);
	}
}
