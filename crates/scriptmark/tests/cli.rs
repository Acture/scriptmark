//! The command line around frozen inputs (P-675): `grade` and `run` write them beside the
//! results, refuse to replace other inputs unless told to, and replay them on request.

use std::path::Path;
use std::process::{Command, Output};

use scriptmark::runner::frozen::Frozen;

const SPEC: &str = r#"
[meta]
name = "clamp"
file = "clamp.py"
function = "clamp"
language = "python"

[[cases]]
name = "clamp"
[[cases.parametrize.args]]
value = "int(-100, 100)"
[[cases.parametrize.args]]
low = "int(-49, -26)"
[[cases.parametrize.args]]
high = "int(26, 49)"
[cases.parametrize]
samples = [[-30, -30, 30]]
[cases.parametrize.random]
count = 4
seed = "random"
[cases.parametrize.oracle]
rhai = "if value < low { low } else if value > high { high } else { value }"
"#;

/// A bench with one template drawing a random seed, and one correct student.
fn bench() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	let write = |name: &str, content: &str| {
		let path = dir.path().join(name);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, content).unwrap();
	};
	write("tests/test_clamp.toml", SPEC);
	write(
		"submissions/alice_clamp.py",
		"def clamp(value, low, high):\n    return max(low, min(value, high))\n",
	);
	dir
}

fn scriptmark(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.args(args)
		.output()
		.unwrap()
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn frozen(dir: &Path) -> Frozen {
	Frozen::load(&dir.join("out/results.cases.json")).unwrap_or_else(|e| panic!("{e}"))
}

const GRADE: [&str; 5] = ["grade", "submissions", "-t", "tests", "-o"];

#[test]
fn test_grade_freezes_its_inputs_and_will_not_silently_replace_them() {
	let dir = bench();
	let dir = dir.path();
	let grade = |extra: &[&str]| {
		let mut args = GRADE.to_vec();
		args.push("out/results.json");
		args.extend(extra);
		scriptmark(dir, &args)
	};

	let first = grade(&[]);
	assert!(first.status.success(), "{}", stderr(&first));
	assert!(stderr(&first).contains("drew seed"), "{}", stderr(&first));
	let inputs = frozen(dir);
	assert_eq!(inputs.specs["clamp"]["clamp"].cases.len(), 5);

	// Grading again would draw other inputs over the ones this batch was graded on.
	let again = grade(&[]);
	assert!(!again.status.success());
	assert!(
		stderr(&again).contains("holds other inputs (case 'clamp' in 'clamp' differs)"),
		"{}",
		stderr(&again)
	);
	assert_eq!(frozen(dir), inputs, "a refused run replaces nothing");

	let replayed = grade(&["--replay", "out/results.cases.json"]);
	assert!(replayed.status.success(), "{}", stderr(&replayed));
	assert!(!stderr(&replayed).contains("drew seed"));
	assert_eq!(frozen(dir).specs, inputs.specs);

	let fresh = grade(&["--fresh"]);
	assert!(fresh.status.success(), "{}", stderr(&fresh));
	assert_ne!(frozen(dir).specs, inputs.specs);

	let both = grade(&["--fresh", "--replay", "out/results.cases.json"]);
	assert!(!both.status.success());
	assert!(
		stderr(&both).contains("cannot be used with"),
		"{}",
		stderr(&both)
	);
}

#[test]
fn test_run_and_the_archive_freeze_the_same_inputs() {
	let dir = bench();
	let dir = dir.path();
	let ran = scriptmark(
		dir,
		&[
			"run",
			"submissions",
			"-t",
			"tests",
			"-o",
			"out/results.json",
		],
	);
	assert!(ran.status.success(), "{}", stderr(&ran));
	let inputs = frozen(dir);

	let graded = scriptmark(
		dir,
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"-o",
			"out/results.json",
			"--replay",
			"out/results.cases.json",
			"--archive",
			"archive",
		],
	);
	assert!(graded.status.success(), "{}", stderr(&graded));
	let archived = Frozen::load(&dir.join("archive/cases_tests.json")).unwrap();
	assert_eq!(archived.specs, inputs.specs);
}

#[test]
fn test_a_bundle_without_templates_freezes_nothing() {
	let dir = tempfile::tempdir().unwrap();
	let dir = dir.path();
	std::fs::create_dir_all(dir.join("tests")).unwrap();
	std::fs::create_dir_all(dir.join("submissions")).unwrap();
	std::fs::write(
		dir.join("tests/test_f.toml"),
		"[meta]\nname = \"f\"\nfile = \"f.py\"\nfunction = \"f\"\nlanguage = \"python\"\n[[cases]]\nname = \"one\"\nargs = [1]\nexpect = 1\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("submissions/alice_f.py"),
		"def f(x):\n    return x\n",
	)
	.unwrap();
	let mut args = GRADE.to_vec();
	args.push("out/results.json");
	let output = scriptmark(dir, &args);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(!dir.join("out/results.cases.json").exists());
}

/// Writing the drawn seed back, as the note says, keeps the same inputs: the next plain
/// run goes ahead.
#[test]
fn test_a_pasted_drawn_seed_grades_again_without_flags() {
	let dir = bench();
	let dir = dir.path();
	let mut args = GRADE.to_vec();
	args.push("out/results.json");
	let first = scriptmark(dir, &args);
	assert!(first.status.success(), "{}", stderr(&first));
	let inputs = frozen(dir);
	let seed = inputs.specs["clamp"]["clamp"].seed.unwrap();

	let spec = dir.join("tests/test_clamp.toml");
	let text = std::fs::read_to_string(&spec).unwrap();
	std::fs::write(
		&spec,
		text.replace("seed = \"random\"", &format!("seed = {seed}")),
	)
	.unwrap();
	let pasted = scriptmark(dir, &args);
	assert!(pasted.status.success(), "{}", stderr(&pasted));
	assert_eq!(
		frozen(dir).specs["clamp"]["clamp"].cases,
		inputs.specs["clamp"]["clamp"].cases
	);
}

/// A refused run grades nobody, so it offers no seed to keep.
#[test]
fn test_a_refused_run_offers_no_seed() {
	let dir = bench();
	let dir = dir.path();
	let mut args = GRADE.to_vec();
	args.push("out/results.json");
	assert!(scriptmark(dir, &args).status.success());
	let refused = scriptmark(dir, &args);
	assert!(!refused.status.success());
	assert!(
		!stderr(&refused).contains("drew seed"),
		"{}",
		stderr(&refused)
	);
}

/// Frozen inputs beside the results are what those results were graded on: a batch with
/// no templates may not leave an earlier batch's there.
#[test]
fn test_a_batch_without_templates_does_not_keep_old_frozen_inputs() {
	let dir = bench();
	let dir = dir.path();
	let mut args = GRADE.to_vec();
	args.push("out/results.json");
	assert!(scriptmark(dir, &args).status.success());
	std::fs::write(
		dir.join("tests/test_clamp.toml"),
		"[meta]\nname = \"clamp\"\nfile = \"clamp.py\"\nfunction = \"clamp\"\nlanguage = \"python\"\n[[cases]]\nname = \"inside\"\nargs = [5, 0, 10]\nexpect = 5\n",
	)
	.unwrap();
	let refused = scriptmark(dir, &args);
	assert!(!refused.status.success());
	assert!(
		stderr(&refused).contains("holds other inputs (spec 'clamp' differs)"),
		"{}",
		stderr(&refused)
	);
	args.push("--fresh");
	let fresh = scriptmark(dir, &args);
	assert!(fresh.status.success(), "{}", stderr(&fresh));
	assert!(!dir.join("out/results.cases.json").exists());
}
