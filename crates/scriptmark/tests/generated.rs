//! Generated cases end to end (P-675): templates are expanded once in `prepare`, their
//! inputs bind in call order for the student and the oracles alike, and anything that
//! cannot be generated or answered is refused before a student runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::models::*;
use scriptmark::runner::frozen::{Frozen, Generation};
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

	/// A spec named `t` for `f` in `lab.py`, with this body.
	fn spec(&self, body: &str) -> TestSpec {
		self.named("t", body)
	}

	fn named(&self, name: &str, body: &str) -> TestSpec {
		let toml = format!(
			"[meta]\nname = \"{name}\"\nfile = \"lab.py\"\nfunction = \"f\"\nlanguage = \"python\"\n{body}"
		);
		load_spec_str(&toml, self.path()).unwrap_or_else(|e| panic!("{e}"))
	}

	fn student(&self, key: &str, code: &str) -> StudentSubmission {
		let path = self.write(&format!("students/{key}/lab.py"), code);
		StudentSubmission::from_files(key, &[path])
	}
}

async fn prepared(specs: Vec<TestSpec>) -> Result<Vec<Bundle>, String> {
	prepared_with(specs, &Generation::fresh()).await
}

async fn prepared_with(
	specs: Vec<TestSpec>,
	generation: &Generation,
) -> Result<Vec<Bundle>, String> {
	prepare(specs, generation, Arc::new(PythonExecutor::new()), 5)
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
	run_with(bundles, students, 4).await
}

async fn run_with(
	bundles: Vec<Bundle>,
	students: &[StudentSubmission],
	concurrency: usize,
) -> Vec<StudentReport> {
	let options = RunOptions {
		concurrency: Some(concurrency),
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

/// What each case was given and how it ended, by case name.
fn evidence(report: &StudentReport) -> Vec<(String, Vec<Value>, TestStatus)> {
	report
		.test_results
		.iter()
		.flat_map(|t| &t.cases)
		.map(|c| {
			let args = c.input.as_ref().map(|i| i.args.clone()).unwrap_or_default();
			(c.case_name.clone(), args, c.status)
		})
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
[cases.parametrize.args]
value = "choice([-100, -75, -25, 0, 25, 75, 100])"
low = "int(-49, -26)"
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
			"[[cases]]\nname = \"s\"\ncheck = \"sorted\"\n[cases.parametrize.args]\na = \"list(int(0, 9), 0, 3)\"\n[cases.parametrize.random]\ncount = 2\n{seed}"
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
		"[[cases]]\nname = \"g\"\nfunction = \"other\"\ntimeout = 3\ncheck = \"sorted\"\n[cases.parametrize.args]\nx = \"list(int(0, 1), 0, 2)\"\n[cases.parametrize.random]\ncount = 2\n",
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
		"[[cases]]\nname = \"d\"\n[cases.parametrize.args]\nx = \"str(0, 3)\"\n[cases.parametrize]\nsamples = [[\"$$5\"], [[\"$$a\", 1]]]\n[cases.parametrize.oracle]\nrhai = \"x\"\n",
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
args = { a = "int(0, 9)", b = "int(0, 9)" }
samples = [[5, 5], [0, 9]]
[cases.parametrize.oracle]
reference = "reference/lab.py"

[[cases]]
name = "draws with rhai and no seed"
[cases.parametrize]
args = { a = "int(0, 9)", b = "int(10, 19)" }
[cases.parametrize.random]
count = 4
[cases.parametrize.oracle]
rhai = "[a, b]"

[[cases]]
name = "both, with a property check"
[cases.parametrize]
args = { a = "int(-5, 5)", b = "int(-5, 5)" }
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
		"[[cases]]\nname = \"x\"\ncheck = \"approx\"\n[cases.parametrize]\nargs = { a = \"int(0, 9)\" }\nsamples = [[1]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
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
		"[[cases]]\nname = \"x\"\n[cases.parametrize]\nargs = { a = \"int(0, 9)\" }\nsamples = [[1]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
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
		"[[cases]]\nname = \"x\"\n[cases.parametrize]\nargs = { a = \"int(0, 9)\" }\nsamples = [[1], [2]]\n[cases.parametrize.oracle]\nreference = \"reference/lab.py\"\n",
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
		"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\n[cases.parametrize.args]\na = \"list(int(0, 1), 0, 2)\"\n[cases.parametrize.random]\ncount = 1\n",
	);
	spec.cases[0].parametrize.as_mut().unwrap().args[0].rule = "int(5, 1)".into();
	let message = refusal(spec).await;
	assert!(message.contains("parameter 'a'"), "{message}");
	assert!(message.contains("greater than"), "{message}");
}

/// CLAMP answered by an echo, so any argument list at all is the right answer to itself.
fn echo(bench: &Bench) -> TestSpec {
	bench.spec(&CLAMP.replace(
		"if value < low { low } else if value > high { high } else { value }",
		"[value, low, high]",
	))
}

const ECHO: &str = "def f(value, low, high):\n    return [value, low, high]\n";

#[tokio::test]
async fn test_replay_grades_the_frozen_rows_as_they_are() {
	let bench = Bench::new();
	let fresh = Frozen::of(&prepared(vec![echo(&bench)]).await.unwrap());
	// Rows no rule could draw, from a generator this build is not: taken as they are.
	let mut value: Value = serde_json::from_str(&fresh.to_json()).unwrap();
	value["specs"]["t"]["clamp"]["generator"] = json!(999);
	value["specs"]["t"]["clamp"]["cases"][3]["args"] = json!([5000, 5000, 5000]);
	let edited = Frozen::from_json(&value.to_string()).unwrap();

	let bundles = prepared_with(vec![echo(&bench)], &Generation::Replay(edited.clone()))
		.await
		.unwrap_or_else(|e| panic!("{e}"));
	assert_eq!(Frozen::of(&bundles).specs, edited.specs);
	let reports = run(bundles, &[bench.student("alice", ECHO)]).await;
	let rows = evidence(&reports[0]);
	let draw0 = rows.iter().find(|(name, ..)| name == "clamp [0]").unwrap();
	assert_eq!(draw0.1, [json!(5000), json!(5000), json!(5000)]);
	assert_eq!(failed(&reports[0]), Vec::<String>::new());
}

#[tokio::test]
async fn test_replay_reruns_a_corrected_oracle_on_the_same_inputs() {
	let bench = Bench::new();
	let wrong = bench.spec(&CLAMP.replace(
		"if value < low { low } else if value > high { high } else { value }",
		"[high, low, value]",
	));
	let frozen = Frozen::of(&prepared(vec![wrong]).await.unwrap());
	let bundles = prepared_with(vec![echo(&bench)], &Generation::Replay(frozen.clone()))
		.await
		.unwrap_or_else(|e| panic!("{e}"));
	let recorded = &frozen.specs["t"]["clamp"].cases;
	assert_eq!(bundles[0].spec.cases.len(), recorded.len());
	for (case, row) in bundles[0].spec.cases.iter().zip(recorded) {
		assert_eq!(case.args, row.args);
		assert_eq!(case.expect, Some(Value::from(row.args.clone())));
	}
}

#[tokio::test]
async fn test_replay_refuses_inputs_it_cannot_honour() {
	let bench = Bench::new();
	let frozen = Frozen::of(&prepared(vec![echo(&bench)]).await.unwrap());
	let replay = Generation::Replay(frozen);
	let refused = |specs: Vec<TestSpec>, needle: &'static str| {
		let replay = &replay;
		async move {
			let message = prepared_with(specs, replay).await.map(|_| ()).unwrap_err();
			assert!(
				message.contains(needle),
				"expected {needle:?} in:\n{message}"
			);
		}
	};

	refused(
		vec![bench.spec(&CLAMP.replace("count = 12", "count = 30"))],
		"the spec says count = 30; the frozen inputs used count = 12",
	)
	.await;
	refused(
		vec![bench.spec(&format!(
			"{CLAMP}\n[[cases]]\nname = \"other\"\ncheck = \"sorted\"\n[cases.parametrize.args]\nx = \"list(int(0, 1), 0, 2)\"\n[cases.parametrize.random]\ncount = 1\n"
		))],
		"case 'other': the frozen inputs have no template by this name",
	)
	.await;
	refused(
		vec![bench.spec("[[cases]]\nname = \"fixed\"\nargs = [1, 2, 3]\nexpect = [1, 2, 3]\n")],
		"the frozen inputs have template 'clamp', which the spec does not",
	)
	.await;
	refused(
		vec![bench.named(
			"u",
			"[[cases]]\nname = \"fixed\"\nargs = [1, 2, 3]\nexpect = [1, 2, 3]\n",
		)],
		"the frozen inputs have spec 't', which this batch does not",
	)
	.await;
}

#[tokio::test]
async fn test_inputs_do_not_depend_on_order_or_concurrency() {
	let bench = Bench::new();
	let second = "[[cases]]\nname = \"pairs\"\ncheck = \"sorted\"\n[cases.parametrize.args]\nxs = \"list(int(0, 9), 0, 4)\"\n[cases.parametrize.random]\ncount = 6\nseed = 3\n";
	let a = || bench.named("a", &format!("{CLAMP}{second}"));
	let b = || bench.named("b", &format!("{second}{CLAMP}"));
	let ab = Frozen::of(&prepared(vec![a(), b()]).await.unwrap());
	let ba = Frozen::of(&prepared(vec![b(), a()]).await.unwrap());
	assert_eq!(ab.specs, ba.specs);
	// Templates in another order, in another spec: each template's inputs are its own.
	assert_eq!(ab.specs["a"], ab.specs["b"]);

	let code = [
		("alice", ECHO),
		(
			"bob",
			"def f(value, low, high):\n    return [high, low, value]\n",
		),
	];
	let students: Vec<_> = code.iter().map(|(k, c)| bench.student(k, c)).collect();
	let mut reversed = students.clone();
	reversed.reverse();
	let one = run_with(prepared(vec![echo(&bench)]).await.unwrap(), &students, 1).await;
	let eight = run_with(prepared(vec![echo(&bench)]).await.unwrap(), &reversed, 8).await;
	for (key, _) in code {
		assert_eq!(
			evidence(report(&one, key)),
			evidence(report(&eight, key)),
			"{key}"
		);
	}
}

#[tokio::test]
async fn test_a_pasted_drawn_seed_reproduces_the_inputs() {
	let bench = Bench::new();
	let drawn = bundle(bench.spec(&CLAMP.replace("seed = 7", "seed = \"random\""))).await;
	let g = &drawn.generated["clamp"];
	assert_eq!(g.seed_source, Some(SeedSource::Drawn));
	let seed = g.seed.unwrap();
	let pasted = bundle(bench.spec(&CLAMP.replace("seed = 7", &format!("seed = {seed}")))).await;
	let p = &pasted.generated["clamp"];
	assert_eq!(
		(p.seed, p.seed_source),
		(Some(seed), Some(SeedSource::Declared))
	);
	assert_eq!(p.cases, g.cases);
}

#[tokio::test]
async fn test_a_failed_seed_draw_is_refused() {
	fn broken() -> Result<u64, String> {
		Err("no entropy here".into())
	}
	let bench = Bench::new();
	let message = prepared_with(
		vec![bench.spec(&CLAMP.replace("seed = 7", "seed = \"random\""))],
		&Generation::Fresh(broken),
	)
	.await
	.map(|_| ())
	.unwrap_err();
	assert!(
		message.contains("case 'clamp': no entropy here"),
		"{message}"
	);
}
