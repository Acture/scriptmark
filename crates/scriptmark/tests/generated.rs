//! Generated cases end to end (P-675): templates are expanded once in `prepare`, their
//! inputs bind in call order for the student and the oracles alike, and anything that
//! cannot be generated or answered is refused before a student runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::models::*;
use scriptmark::runner::generation::{Origin, SeedSource};
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::{Bundle, prepare};
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::load_spec_str;
use serde_json::{Value, json};

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

	/// A spec for `f` in `lab.py`, with this body.
	fn spec(&self, body: &str) -> TestSpec {
		let toml = format!(
			"[meta]\nname = \"t\"\nfile = \"lab.py\"\nfunction = \"f\"\nlanguage = \"python\"\n{body}"
		);
		load_spec_str(&toml, self.path()).unwrap_or_else(|e| panic!("{e}"))
	}

	fn student(&self, key: &str, code: &str) -> StudentSubmission {
		let path = self.write(&format!("students/{key}/lab.py"), code);
		StudentSubmission::from_files(key, &[path])
	}
}

async fn prepared(specs: Vec<TestSpec>) -> Result<Vec<Bundle>, String> {
	prepare(specs, Arc::new(PythonExecutor::new()), 5)
		.await
		.map_err(|e| e.to_string())
}

async fn bundle(spec: TestSpec) -> Bundle {
	prepared(vec![spec])
		.await
		.unwrap_or_else(|e| panic!("{e}"))
		.remove(0)
}

async fn refusal(spec: TestSpec) -> String {
	prepared(vec![spec]).await.map(|_| ()).unwrap_err()
}

async fn run(bundles: Vec<Bundle>, students: &[StudentSubmission]) -> Vec<StudentReport> {
	let options = RunOptions {
		concurrency: Some(4),
		..Default::default()
	};
	run_all(
		students,
		bundles.into(),
		Arc::new(PythonExecutor::new()),
		&options,
	)
	.await
}

/// The cases a student failed, by name.
fn failed(report: &StudentReport) -> Vec<String> {
	report
		.test_results
		.iter()
		.flat_map(|t| &t.cases)
		.filter(|c| c.status != TestStatus::Passed)
		.map(|c| c.case_name.clone())
		.collect()
}

fn report<'a>(reports: &'a [StudentReport], id: &str) -> &'a StudentReport {
	reports
		.iter()
		.find(|r| r.student_id == id)
		.unwrap_or_else(|| panic!("no report for '{id}'"))
}

/// `clamp(value, low, high)`: parameters written in reverse alphabetical order, samples on
/// and below the bounds, and draws whose `value` never equals a bound.
const CLAMP: &str = r#"
[[cases]]
name = "clamp"
[[cases.parametrize.args]]
value = "choice([-100, -75, -25, 0, 25, 75, 100])"
[[cases.parametrize.args]]
low = "int(-49, -26)"
[[cases.parametrize.args]]
high = "int(26, 49)"
[cases.parametrize]
samples = [[-30, -30, 30], [30, -30, 30], [-100, -30, 30]]
[cases.parametrize.random]
count = 12
seed = 7
[cases.parametrize.oracle]
rhai = "if value < low { low } else if value > high { high } else { value }"
"#;

/// The student gets its arguments in the order `args` lists them, and the oracle binds each
/// name to the same value: an echo exposes any disagreement, whatever the function does.
#[tokio::test]
async fn test_arguments_bind_in_call_order_for_the_student_and_the_oracle() {
	let bench = Bench::new();
	let spec = bench.spec(&CLAMP.replace(
		"if value < low { low } else if value > high { high } else { value }",
		"[value, low, high]",
	));
	let students = [
		bench.student(
			"alice",
			"def f(value, low, high):\n    return [value, low, high]\n",
		),
		// Takes its parameters in alphabetical order.
		bench.student(
			"alpha",
			"def f(high, low, value):\n    return [value, low, high]\n",
		),
	];
	let reports = run(vec![bundle(spec).await], &students).await;
	assert_eq!(failed(report(&reports, "alice")), Vec::<String>::new());
	// Every case but sample 1, whose `value` equals its `high`, so the swap goes unseen.
	let alpha = failed(report(&reports, "alpha"));
	assert_eq!(alpha.len(), 14, "{alpha:?}");
	assert!(!alpha.contains(&"clamp [sample 1]".to_string()));
}

#[tokio::test]
async fn test_the_bundle_records_each_templates_inputs() {
	let bench = Bench::new();
	let b = bundle(bench.spec(CLAMP)).await;
	let g = &b.generated["clamp"];
	assert_eq!(
		(g.seed, g.seed_source),
		(Some(7), Some(SeedSource::Declared))
	);
	assert_eq!(g.cases.len(), 15);
	assert_eq!(g.cases[2].origin, Origin::Sample(2));
	assert_eq!(g.cases[3].origin, Origin::Draw(0));
	// The cases that run are the recorded ones, in the recorded order.
	let run: Vec<(&str, &Vec<Value>)> = b
		.spec
		.cases
		.iter()
		.map(|c| (c.name.as_str(), &c.args))
		.collect();
	let recorded: Vec<(&str, &Vec<Value>)> =
		g.cases.iter().map(|c| (c.name.as_str(), &c.args)).collect();
	assert_eq!(run, recorded);

	let seeds = |seed: &str| {
		bench.spec(&format!(
			"[[cases]]\nname = \"s\"\ncheck = \"sorted\"\n[[cases.parametrize.args]]\na = \"list(int(0, 9), 0, 3)\"\n[cases.parametrize.random]\ncount = 2\n{seed}"
		))
	};
	let default = bundle(seeds("")).await;
	let g = &default.generated["s"];
	assert_eq!(
		(g.seed, g.seed_source),
		(Some(0), Some(SeedSource::Default))
	);
	let drawn = bundle(seeds("seed = \"random\"\n")).await;
	let g = &drawn.generated["s"];
	assert_eq!(g.seed_source, Some(SeedSource::Drawn));
	assert!(g.seed.unwrap() < 1 << 53);
}

#[tokio::test]
async fn test_a_generated_case_keeps_its_templates_target_and_timeout() {
	let bench = Bench::new();
	let b = bundle(bench.spec(
		"[[cases]]\nname = \"g\"\nfunction = \"other\"\ntimeout = 3\ncheck = \"sorted\"\n[[cases.parametrize.args]]\nx = \"list(int(0, 1), 0, 2)\"\n[cases.parametrize.random]\ncount = 2\n",
	))
	.await;
	for case in &b.spec.cases {
		assert_eq!(case.function.as_deref(), Some("other"));
		assert_eq!(case.timeout, Some(3));
		assert!(case.parametrize.is_none());
	}
}

#[tokio::test]
async fn test_a_dollar_literal_reaches_the_oracle_and_the_student_alike() {
	let bench = Bench::new();
	let spec = bench.spec(
		"[[cases]]\nname = \"d\"\n[[cases.parametrize.args]]\nx = \"str(0, 3)\"\n[cases.parametrize]\nsamples = [[\"$$5\"], [[\"$$a\", 1]]]\n[cases.parametrize.oracle]\nrhai = \"x\"\n",
	);
	let b = bundle(spec).await;
	assert_eq!(b.spec.cases[0].expect, Some(json!("$5")));
	assert_eq!(b.spec.cases[1].expect, Some(json!(["$a", 1])));
	let reports = run(
		vec![b],
		&[bench.student("echo", "def f(x):\n    return x\n")],
	)
	.await;
	assert_eq!(failed(&reports[0]), Vec::<String>::new());
}

/// Samples, draws, a seed and a reference combine in one bundle, and each may be absent.
#[tokio::test]
async fn test_samples_draws_seeds_and_references_combine() {
	let bench = Bench::new();
	bench.write(
		"reference/lab.py",
		"def f(a, b):\n    return sorted([a, b])\n",
	);
	let spec = bench.spec(
		r#"
[[cases]]
name = "fixed"
args = [1, 2]
expect = [1, 2]

[[cases]]
name = "samples with a reference"
[cases.parametrize]
args = [{ a = "int(0, 9)" }, { b = "int(0, 9)" }]
samples = [[5, 5], [0, 9]]
[cases.parametrize.oracle]
reference = "reference/lab.py"

[[cases]]
name = "draws with rhai and no seed"
[cases.parametrize]
args = [{ a = "int(0, 9)" }, { b = "int(10, 19)" }]
[cases.parametrize.random]
count = 4
[cases.parametrize.oracle]
rhai = "[a, b]"

[[cases]]
name = "both, with a property check"
[cases.parametrize]
args = [{ a = "int(-5, 5)" }, { b = "int(-5, 5)" }]
samples = [[-3, 3]]
[cases.parametrize.random]
count = 4
seed = "random"
[cases.parametrize.oracle]
check = "sorted"
"#,
	);
	let reports = run(
		vec![bundle(spec).await],
		&[
			bench.student("alice", "def f(a, b):\n    return sorted([a, b])\n"),
			bench.student("bob", "def f(a, b):\n    return [b, a]\n"),
		],
	)
	.await;
	assert_eq!(report(&reports, "alice").total_passed(), 12);
	let bob = failed(report(&reports, "bob"));
	for template in [
		"fixed",
		"samples with a reference",
		"draws with rhai and no seed",
		"both, with a property check",
	] {
		assert!(
			bob.iter().any(|c| c.starts_with(template)),
			"{template}: {bob:?}"
		);
	}
}

#[tokio::test]
async fn test_two_specs_with_one_name_are_refused() {
	let bench = Bench::new();
	let spec = bench.spec("[[cases]]\nname = \"x\"\nargs = [1]\nexpect = 1\n");
	let message = prepared(vec![spec.clone(), spec])
		.await
		.map(|_| ())
		.unwrap_err();
	assert!(message.contains("two specs are named 't'"), "{message}");
}

#[tokio::test]
async fn test_an_oracle_answer_that_cannot_fit_its_checker_is_refused() {
	let bench = Bench::new();
	bench.write("reference/lab.py", "def f(a):\n    return [a]\n");
	let message = refusal(bench.spec(
		"[[cases]]\nname = \"x\"\ncheck = \"approx\"\n[cases.parametrize]\nargs = [{ a = \"int(0, 9)\" }]\nsamples = [[1]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
	))
	.await;
	assert!(
		message.contains("the approx checker needs a number"),
		"{message}"
	);
}

#[tokio::test]
async fn test_a_reference_answer_holding_none_is_an_answer() {
	let bench = Bench::new();
	bench.write("reference/lab.py", "def f(a):\n    return [a, None]\n");
	let b = bundle(bench.spec(
		"[[cases]]\nname = \"x\"\n[cases.parametrize]\nargs = [{ a = \"int(0, 9)\" }]\nsamples = [[1]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
	))
	.await;
	assert_eq!(b.spec.cases[0].expect, Some(json!([1, null])));
	let reports = run(
		vec![b],
		&[bench.student("alice", "def f(a):\n    return [a, None]\n")],
	)
	.await;
	assert_eq!(failed(&reports[0]), Vec::<String>::new());
}

#[tokio::test]
async fn test_a_failed_oracle_is_reported_once() {
	let bench = Bench::new();
	bench.write("reference/lab.py", "def f(a):\n    print(a)\n");
	let message = refusal(bench.spec(
		"[[cases]]\nname = \"x\"\n[cases.parametrize]\nargs = [{ a = \"int(0, 9)\" }]\nsamples = [[1], [2]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
	))
	.await;
	assert!(message.contains("returned None"), "{message}");
	assert!(!message.contains("nothing to judge"), "{message}");
	assert!(!message.contains("which is missing"), "{message}");
}

#[tokio::test]
async fn test_a_bad_rule_in_a_hand_built_spec_is_refused_not_null() {
	let bench = Bench::new();
	let mut spec = bench.spec(
		"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\n[[cases.parametrize.args]]\na = \"list(int(0, 1), 0, 2)\"\n[cases.parametrize.random]\ncount = 1\n",
	);
	spec.cases[0].parametrize.as_mut().unwrap().args[0].rule = "int(5, 1)".into();
	let message = refusal(spec).await;
	assert!(message.contains("parameter 'a'"), "{message}");
	assert!(message.contains("greater than"), "{message}");
}
