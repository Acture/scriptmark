//! The harness, observed directly through `Executor::run`: what a unit reports, before
//! anything is judged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark_core::models::Target;
use scriptmark_core::runner::executor::{
	CallPlan, CheckObservation, Executor, Exit, InProcessCheck, Outcome, Phase, ScriptRun,
	StopReason, Subject, UnitObservation, UnitPlan,
};
use scriptmark_core::runner::python::PythonExecutor;
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
	let path = dir.join(name);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).unwrap();
	}
	std::fs::write(&path, content).unwrap();
	path
}

fn function(name: &str, args: Vec<Value>) -> CallPlan {
	call(
		Target::Function {
			name: name.to_string(),
		},
		args,
	)
}

fn call(target: Target, args: Vec<Value>) -> CallPlan {
	CallPlan {
		target,
		args,
		stdin: None,
		timeout: 5,
		id: None,
		files: Vec::new(),
		check: None,
	}
}

fn unit(file: &Path) -> UnitPlan {
	UnitPlan {
		subject: Subject::Student,
		functions: scriptmark_core::matching::Functions::default(),
		file: file.to_path_buf(),
		script: None,
		imports: Vec::new(),
		vars: Arc::new(BTreeMap::new()),
		data_files: Vec::new(),
		bundle: file.parent().unwrap().to_path_buf(),
		shared: Vec::new(),
		memory_mb: 64,
		memory_basis: None,
		allowed_imports: Vec::new(),
		load_timeout: 5,
		setup: Vec::new(),
		steps: Vec::new(),
	}
}

async fn run(plan: &UnitPlan) -> UnitObservation {
	PythonExecutor::new().run(plan).await
}

fn returned(outcome: &Outcome) -> &Value {
	match outcome {
		Outcome::Returned { value, .. } => value,
		other => panic!("expected a returned value, got {other:?}"),
	}
}

#[tokio::test]
async fn test_printing_and_stdlib_imports_do_not_break_the_protocol() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import random, csv, datetime, json, pathlib, statistics, decimal\nprint('loading')\ndef add(a, b):\n    print('debug', a, b)\n    return a + b\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("add", vec![json!(1), json!(2)])];
	let obs = run(&plan).await;
	assert!(obs.ready && obs.done, "{obs:?}");
	assert!(matches!(
		obs.load.as_ref().unwrap().outcome,
		Outcome::Returned { .. }
	));
	assert_eq!(returned(&obs.steps[0].outcome), &json!(3));
	assert_eq!(obs.steps[0].stdout, "debug 1 2\n");
	assert_eq!(obs.exit, Exit::Code(0));
}

#[tokio::test]
async fn test_the_guard_still_refuses_a_student_import() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def sneaky():\n    import os\n    return os.getcwd()\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("sneaky", vec![])];
	let obs = run(&plan).await;
	match &obs.steps[0].outcome {
		Outcome::Raised(e) => {
			assert!(e.is_a("ImportError"));
			assert!(e.message.contains("'os' is not allowed"));
		}
		other => panic!("{other:?}"),
	}
}

#[tokio::test]
async fn test_a_timeout_is_recorded_even_through_a_bare_except() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def spin():\n    try:\n        while True:\n            pass\n    except:\n        return -1\n\ndef ok():\n    return 1\n",
	);
	let mut plan = unit(&student);
	let mut spin = function("spin", vec![]);
	spin.timeout = 1;
	plan.steps = vec![spin, function("ok", vec![])];
	let obs = run(&plan).await;
	assert!(
		matches!(obs.steps[0].outcome, Outcome::Timeout {}),
		"{obs:?}"
	);
	assert_eq!(
		returned(&obs.steps[1].outcome),
		&json!(1),
		"later steps still run"
	);
}

/// A call that defeats its own timer is given up on by the teacher side, which reports it
/// and everything else: the student side is ended, and nothing after it runs.
#[tokio::test]
async fn test_a_call_that_never_yields_is_given_up_on_and_nothing_after_it_runs() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		// Never returns to the interpreter, where the timer would fire.
		"import itertools\n\ndef forever():\n    return sum(itertools.count())\n\ndef ok():\n    return 1\n",
	);
	let mut plan = unit(&student);
	plan.load_timeout = 1;
	let mut forever = function("forever", vec![]);
	forever.timeout = 1;
	let mut ok = function("ok", vec![]);
	ok.timeout = 1;
	plan.steps = vec![forever, ok];
	let started = std::time::Instant::now();
	let obs = run(&plan).await;
	assert!(
		started.elapsed() < std::time::Duration::from_secs(10),
		"given up on at its own deadline, not the unit's"
	);
	assert!(obs.done && obs.teacher_exit == Exit::Code(0), "{obs:?}");
	assert_eq!(obs.exit, Exit::Stopped);
	assert_eq!(
		obs.steps.len(),
		1,
		"the hung call is reported; the next never ran"
	);
	assert!(matches!(obs.steps[0].outcome, Outcome::Timeout {}));
	let stopped = obs.stopped.as_ref().unwrap();
	assert_eq!(
		(stopped.phase, stopped.index, stopped.reason),
		(Phase::Step, 0, StopReason::Deadline)
	);
}

#[tokio::test]
async fn test_a_shared_object_is_built_once_and_observed_by_attribute() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"bank.py",
		"class Account:\n    def __init__(self, balance):\n        self.balance = balance\n    def deposit(self, n):\n        self.balance += n\n        return self.balance\n",
	);
	let mut plan = unit(&student);
	let mut make = function("Account", vec![json!(100)]);
	make.id = Some("acct".into());
	plan.setup = vec![make];
	plan.steps = vec![
		call(
			Target::Method {
				object: "acct".into(),
				name: "deposit".into(),
			},
			vec![json!(50)],
		),
		call(
			Target::Attribute {
				object: "acct".into(),
				name: "balance".into(),
			},
			vec![],
		),
		call(
			Target::Attribute {
				object: "acct".into(),
				name: "owner".into(),
			},
			vec![],
		),
	];
	let obs = run(&plan).await;
	assert_eq!(obs.setup.len(), 1);
	assert_eq!(returned(&obs.steps[0].outcome), &json!(150));
	assert_eq!(returned(&obs.steps[1].outcome), &json!(150));
	assert!(matches!(obs.steps[2].outcome, Outcome::Missing { .. }));
}

#[tokio::test]
async fn test_files_are_staged_in_and_observed_out_of_a_private_directory() {
	let dir = tempfile::tempdir().unwrap();
	let submission = dir.path().join("submission");
	let student = write(
		&submission,
		"lab.py",
		"from pathlib import Path\n\ndef read_here():\n    return (Path(__file__).parent / 'data' / 'in.txt').read_text()\n\ndef read_cwd():\n    return open('data/in.txt').read()\n\ndef save(text):\n    with open('out.txt', 'w') as fh:\n        fh.write(text)\n",
	);
	let data = write(dir.path(), "spec/data/in.txt", "hello\n");
	let mut plan = unit(&student);
	plan.data_files = vec![(data.parent().unwrap().to_path_buf(), "data".into())];
	let mut save = function("save", vec![json!("written\n")]);
	save.files = vec!["out.txt".into(), "absent.txt".into()];
	plan.steps = vec![
		function("read_here", vec![]),
		function("read_cwd", vec![]),
		save,
	];
	let obs = run(&plan).await;
	assert_eq!(returned(&obs.steps[0].outcome), &json!("hello\n"));
	assert_eq!(returned(&obs.steps[1].outcome), &json!("hello\n"));
	assert_eq!(
		obs.steps[2].files.get("out.txt"),
		Some(&Some("written\n".to_string()))
	);
	assert_eq!(obs.steps[2].files.get("absent.txt"), Some(&None));
	let left: Vec<_> = std::fs::read_dir(&submission).unwrap().collect();
	assert_eq!(left.len(), 1, "nothing written beside the submission");
}

#[tokio::test]
async fn test_a_script_reads_its_stdin_every_way_and_may_exit_cleanly() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"io.py",
		"import sys\nname = input('name? ')\nrest = sys.stdin.read()\nprint('hi', name, rest.split())\nsys.exit(0)\n",
	);
	let mut plan = unit(&student);
	plan.script = Some(ScriptRun {
		stdin: Some("ada\n1 2\n".into()),
		timeout: 5,
		files: Vec::new(),
	});
	let obs = run(&plan).await;
	assert!(
		matches!(obs.steps[0].outcome, Outcome::Returned { .. }),
		"{obs:?}"
	);
	assert_eq!(obs.steps[0].stdout, "name? hi ada ['1', '2']\n");

	let failing = write(dir.path(), "bad.py", "import sys\nsys.exit(3)\n");
	let mut plan = unit(&failing);
	plan.script = Some(ScriptRun {
		stdin: None,
		timeout: 5,
		files: Vec::new(),
	});
	let obs = run(&plan).await;
	assert!(matches!(&obs.steps[0].outcome, Outcome::Raised(e) if e.is_a("SystemExit")));
}

#[tokio::test]
async fn test_exit_at_import_is_the_students_raise_not_a_silent_death() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"choice = input()\nif choice == '0':\n    exit()\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("f", vec![])];
	let obs = run(&plan).await;
	assert!(obs.done);
	assert!(matches!(
		&obs.load.as_ref().unwrap().outcome,
		Outcome::Raised(e) if e.is_a("SystemExit")
	));
	assert!(obs.steps.is_empty());
}

#[tokio::test]
async fn test_values_that_cannot_be_serialised_are_reported_not_crashed_on() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def numbers():\n    return {10, 9, 100}\n\nclass Loud:\n    def __str__(self):\n        print('side effect')\n\ndef mixed():\n    return {1, 'a'}\n\ndef collide():\n    return {1: 'a', '1': 'b'}\n\ndef loud():\n    return Loud()\n\ndef cyclic():\n    x = []\n    x.append(x)\n    return x\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![
		function("numbers", vec![]),
		function("mixed", vec![]),
		function("collide", vec![]),
		function("loud", vec![]),
		function("cyclic", vec![]),
	];
	let obs = run(&plan).await;
	assert_eq!(returned(&obs.steps[0].outcome), &json!([9, 10, 100]));
	assert_eq!(returned(&obs.steps[1].outcome), &json!(["a", 1]));
	assert!(matches!(obs.steps[2].outcome, Outcome::Unserialisable(_)));
	assert_eq!(returned(&obs.steps[3].outcome), &json!("<Loud>"));
	assert_eq!(returned(&obs.steps[4].outcome), &json!(["<cycle>"]));
}

#[tokio::test]
async fn test_in_process_checks_report_verdicts_rejections_and_errors() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(dir.path(), "lab.py", "def f(x):\n    return x\n");
	let teacher = write(
		dir.path(),
		"teacher.py",
		"def close(result, expected):\n    return abs(result - expected) < 1, 'off by more than one'\n\ndef strict(result, expected):\n    assert isinstance(result, str), 'expected a string'\n    return True\n\ndef broken(result, expected):\n    return result.nope\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let checked = |name: &str, arg: Value| {
		let mut c = function("f", vec![arg]);
		c.check = Some(InProcessCheck {
			function: name.to_string(),
			expected: Some(json!(10)),
		});
		c
	};
	plan.steps = vec![
		checked("close", json!(10.5)),
		checked("strict", json!(3)),
		checked("broken", json!(3)),
	];
	let obs = run(&plan).await;
	assert_eq!(
		obs.checks[&0],
		CheckObservation::Verdict {
			pass: true,
			message: "off by more than one".into()
		}
	);
	assert_eq!(
		obs.checks[&1],
		CheckObservation::Rejected {
			message: "expected a string".into()
		}
	);
	assert!(matches!(&obs.checks[&2], CheckObservation::Error(e) if e.is_a("AttributeError")));
}

#[tokio::test]
async fn test_refs_resolve_live_and_a_missing_one_is_reported() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def echo(x):\n    return x\n\ndef boom():\n    raise ValueError('no')\n",
	);
	let mut plan = unit(&student);
	let mut boom = function("boom", vec![]);
	boom.id = Some("never".into());
	plan.vars = Arc::new(BTreeMap::from([("LIMIT".to_string(), json!(7))]));
	plan.steps = vec![
		function("echo", vec![json!("$LIMIT")]),
		function("echo", vec![json!("$$5")]),
		boom,
		function("echo", vec![json!("$never")]),
	];
	let obs = run(&plan).await;
	assert_eq!(returned(&obs.steps[0].outcome), &json!(7));
	assert_eq!(returned(&obs.steps[1].outcome), &json!("$5"));
	assert!(matches!(&obs.steps[2].outcome, Outcome::Raised(e) if e.is_a("Exception")));
	assert!(matches!(&obs.steps[3].outcome, Outcome::Unresolved { name } if name == "never"));
}

#[tokio::test]
async fn test_a_teacher_module_that_fails_to_import_is_fatal_before_ready() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(dir.path(), "lab.py", "def f():\n    return 1\n");
	let teacher = write(
		dir.path(),
		"teacher.py",
		"raise RuntimeError('broken helper')\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	plan.steps = vec![function("f", vec![])];
	let obs = run(&plan).await;
	assert!(!obs.ready);
	assert_eq!(obs.fatal.as_ref().unwrap().stage, "teacher_import");
}

#[tokio::test]
async fn test_inspect_reports_only_what_a_teacher_module_defines() {
	let dir = tempfile::tempdir().unwrap();
	write(dir.path(), "helpers/sibling.py", "BASE = 2\n");
	let teacher = write(
		dir.path(),
		"helpers/teacher.py",
		"import csv\nfrom pathlib import Path\nfrom collections import deque\nfrom sibling import BASE\nDATA = Path(__file__).parent / 'x.csv'\n\ndef make(n, scale=BASE):\n    return n * scale\n",
	);
	let spec: scriptmark_core::models::TestSpec = toml::from_str(&format!(
		"[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\nimports = [{:?}]\n[[cases]]\nname = \"x\"\nexpect = 1\n",
		teacher.to_string_lossy()
	))
	.unwrap();
	let runtime = PythonExecutor::new().inspect(&spec, 5).await.unwrap();
	let names: Vec<&str> = runtime.exports.keys().map(String::as_str).collect();
	assert_eq!(names, ["BASE", "DATA", "make"]);
	let params = runtime.exports["make"].params.as_deref().unwrap();
	let described: Vec<(&str, bool)> = params
		.iter()
		.map(|p| (p.name.as_str(), p.default))
		.collect();
	assert_eq!(described, [("n", false), ("scale", true)]);
}

#[tokio::test]
async fn test_a_raising_property_is_that_steps_exception_not_a_harness_crash() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"bank.py",
		"class Account:\n    reads = 0\n    def __init__(self):\n        self.history = []\n    @property\n    def last(self):\n        Account.reads += 1\n        return self.history[-1]\n    @property\n    def typo(self):\n        return self.histroy\n    def __getattr__(self, name):\n        if name == 'dynamic':\n            raise KeyError(name)\n        raise AttributeError(name)\n    def count(self):\n        return Account.reads\n",
	);
	let mut plan = unit(&student);
	let mut make = function("Account", vec![]);
	make.id = Some("acct".into());
	plan.setup = vec![make];
	let on = |kind: &str, name: &str| {
		let object = "acct".to_string();
		let name = name.to_string();
		call(
			match kind {
				"attribute" => Target::Attribute { object, name },
				_ => Target::Method { object, name },
			},
			vec![],
		)
	};
	plan.steps = vec![
		on("attribute", "last"),
		on("attribute", "typo"),
		on("method", "dynamic"),
		on("attribute", "nowhere"),
		on("method", "count"),
	];
	let obs = run(&plan).await;
	assert!(obs.fatal.is_none(), "{obs:?}");
	assert!(matches!(&obs.steps[0].outcome, Outcome::Raised(e) if e.is_a("IndexError")));
	assert!(
		matches!(&obs.steps[1].outcome, Outcome::Raised(e) if e.is_a("AttributeError")),
		"a typo inside a property is the property raising, not a missing attribute"
	);
	assert!(matches!(&obs.steps[2].outcome, Outcome::Raised(e) if e.is_a("KeyError")));
	assert!(matches!(&obs.steps[3].outcome, Outcome::Raised(e) if e.is_a("AttributeError")));
	assert_eq!(
		returned(&obs.steps[4].outcome),
		&json!(1),
		"the property was read once, inside its own call"
	);
}

#[tokio::test]
async fn test_nothing_a_student_returns_or_prints_can_crash_the_record_channel() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"class Broke(Exception):\n    def __str__(self):\n        return 'n=' + 1\n\ndef bad_message():\n    raise Broke()\n\ndef big():\n    return 2 ** 1100\n\ndef huge():\n    return 10 ** 5000\n\ndef lone():\n    print('\\ud800')\n    return '\\udc80'\n\ndef blocked(path):\n    open('out', 'w').write('a file, not a directory')\n",
	);
	let mut plan = unit(&student);
	let mut blocked = function("blocked", vec![json!("out/x.txt")]);
	blocked.files = vec!["out/x.txt".into()];
	plan.steps = vec![
		function("bad_message", vec![]),
		function("big", vec![]),
		function("huge", vec![]),
		function("lone", vec![]),
		blocked,
	];
	let obs = run(&plan).await;
	assert!(
		obs.fatal.is_none() && obs.protocol_error.is_none(),
		"{obs:?}"
	);
	assert!(matches!(&obs.steps[0].outcome, Outcome::Raised(e) if e.type_name == "Broke"));
	let big = returned(&obs.steps[1].outcome);
	assert!(
		big["$bigint"].as_str().unwrap().starts_with("1358"),
		"{big}"
	);
	assert!(returned(&obs.steps[2].outcome)["$bigint"].is_string());
	assert_eq!(returned(&obs.steps[3].outcome), &json!("?"));
	assert_eq!(obs.steps[4].files.get("out/x.txt"), Some(&None));
}

#[tokio::test]
async fn test_a_submission_cannot_shadow_what_the_harness_imports() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"json.py",
		"def dumps(*a, **k):\n    raise RuntimeError('shadowed')\n\ndef f():\n    return [1, 2]\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("f", vec![])];
	let obs = run(&plan).await;
	assert!(obs.ready && obs.done, "{obs:?}");
	assert_eq!(returned(&obs.steps[0].outcome), &json!([1, 2]));

	let upper = write(dir.path(), "LAB5.PY", "def f():\n    return 5\n");
	let mut plan = unit(&upper);
	plan.steps = vec![function("f", vec![])];
	assert_eq!(returned(&run(&plan).await.steps[0].outcome), &json!(5));
}

#[tokio::test]
async fn test_a_script_sees_itself_as_argv_and_a_real_stdout() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"io.py",
		"import sys\nsys.stdout.reconfigure(encoding='utf-8')\nprint(len(sys.argv), sys.argv[0].endswith('io.py'), flush=True)\nsys.stdout.buffer.write(b'raw\\n')\nexit(False)\n",
	);
	let mut plan = unit(&student);
	plan.script = Some(ScriptRun {
		stdin: None,
		timeout: 5,
		files: Vec::new(),
	});
	let obs = run(&plan).await;
	assert!(
		matches!(obs.steps[0].outcome, Outcome::Returned { .. }),
		"{obs:?}"
	);
	assert_eq!(obs.steps[0].stdout, "1 True\nraw\n");
}

#[tokio::test]
async fn test_checker_contract_and_missing_dependencies_are_reported() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def f():\n    return 1\n\ndef boom():\n    raise ValueError('no')\n",
	);
	let teacher = write(
		dir.path(),
		"teacher.py",
		"def numeric(result, expected):\n    return 1\n\ndef uses(result, expected, made, tol=0.5, **rest):\n    return True\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let checked = |fn_name: &str, check: &str| {
		let mut c = function(fn_name, vec![]);
		c.check = Some(InProcessCheck {
			function: check.to_string(),
			expected: None,
		});
		c
	};
	let mut producer = function("boom", vec![]);
	producer.id = Some("made".into());
	plan.steps = vec![checked("f", "numeric"), producer, checked("f", "uses")];
	let obs = run(&plan).await;
	assert!(
		matches!(&obs.checks[&0], CheckObservation::Error(e) if e.type_name == "CheckerContract")
	);
	assert_eq!(
		obs.checks[&2],
		CheckObservation::Unresolved {
			name: "made".into()
		}
	);
}

#[tokio::test]
async fn test_inspect_names_duplicates_and_the_removed_decorator() {
	let dir = tempfile::tempdir().unwrap();
	let a = write(dir.path(), "a.py", "def helper():\n    return 'a'\n");
	let b = write(dir.path(), "b.py", "def helper():\n    return 'b'\n");
	let spec_of = |imports: &[&PathBuf]| -> scriptmark_core::models::TestSpec {
		toml::from_str(&format!(
			"[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\nimports = {:?}\n[[cases]]\nname = \"x\"\nexpect = 1\n",
			imports.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()
		))
		.unwrap()
	};
	let runtime = PythonExecutor::new()
		.inspect(&spec_of(&[&a, &b]), 5)
		.await
		.unwrap();
	assert_eq!(runtime.duplicates["helper"].len(), 2);

	let decorated = write(
		dir.path(),
		"decorated.py",
		"@checker('f')\ndef check_f(result, expected):\n    return True\n",
	);
	let err = PythonExecutor::new()
		.inspect(&spec_of(&[&decorated]), 5)
		.await
		.unwrap_err();
	assert!(err.contains("check = { function"), "{err}");
}

#[tokio::test]
async fn test_a_reference_runs_as_teacher_code() {
	let dir = tempfile::tempdir().unwrap();
	let reference = write(
		dir.path(),
		"ref.py",
		"import os\n\ndef f():\n    return os.sep\n",
	);
	let mut plan = unit(&reference);
	plan.subject = Subject::Reference;
	plan.steps = vec![function("f", vec![])];
	assert_eq!(returned(&run(&plan).await.steps[0].outcome), &json!("/"));
}

#[tokio::test]
async fn test_a_student_who_rewraps_or_closes_stdout_is_still_heard() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import sys, io\n\nKEEP = []\n\ndef rewrap():\n    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8')\n    print('hi')\n\ndef kept():\n    KEEP.append(io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8'))\n    sys.stdout = KEEP[0]\n    print('hi')\n\ndef detach():\n    print('hi', flush=True)\n    sys.stdout.detach()\n\ndef close():\n    print('hi')\n    sys.stdout.close()\n",
	);
	let mut plan = unit(&student);
	plan.steps = ["rewrap", "kept", "detach", "close"]
		.iter()
		.map(|f| function(f, vec![]))
		.collect();
	let obs = run(&plan).await;
	assert!(obs.fatal.is_none() && obs.done, "{obs:?}");
	for step in &obs.steps {
		assert_eq!(step.stdout, "hi\n", "{step:?}");
	}
}

#[tokio::test]
async fn test_serialisation_edges_found_in_rereview() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def neg():\n    return -10 ** 5000\n\ndef zero_float_exit():\n    raise SystemExit(0.0)\n\ndef big_list():\n    return list(range(1_500_000))\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def length(result, expected):\n    return len(result) == 1_500_000\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	// 1.5 million integers, and the harness's copies of them for the checker.
	plan.memory_mb = 512;
	let mut big = function("big_list", vec![]);
	big.check = Some(InProcessCheck {
		function: "length".into(),
		expected: None,
	});
	plan.steps = vec![
		function("neg", vec![]),
		function("zero_float_exit", vec![]),
		big,
	];
	let obs = run(&plan).await;
	assert!(
		obs.fatal.is_none() && obs.protocol_error.is_none(),
		"{obs:?}"
	);
	let neg = returned(&obs.steps[0].outcome)["$bigint"]
		.as_str()
		.unwrap()
		.to_string();
	assert!(
		neg.starts_with("-0x"),
		"exact and signed past the digit limit: {neg}"
	);
	assert!(matches!(&obs.steps[1].outcome, Outcome::Raised(e) if e.is_a("SystemExit")));
	assert!(returned(&obs.steps[2].outcome)["$too_large"].is_string());
	assert_eq!(
		obs.checks[&2],
		CheckObservation::Verdict {
			pass: true,
			message: String::new()
		},
		"a value too large to report is still judged live"
	);
}

#[tokio::test]
async fn test_a_script_leaves_no_bytecode_and_can_import_a_staged_helper() {
	let dir = tempfile::tempdir().unwrap();
	let helper = write(
		dir.path(),
		"spec/helper.py",
		"def twice(x):\n    return 2 * x\n",
	);
	let student = write(
		dir.path(),
		"io.py",
		"import os\nimport helper\nprint(sorted(os.listdir('.')), helper.twice(4))\n",
	);
	let mut plan = unit(&student);
	plan.allowed_imports = vec!["os".into(), "helper".into()];
	plan.data_files = vec![(helper, "helper.py".into())];
	plan.script = Some(ScriptRun {
		stdin: None,
		timeout: 5,
		files: Vec::new(),
	});
	let obs = run(&plan).await;
	assert!(
		matches!(obs.steps[0].outcome, Outcome::Returned { .. }),
		"{obs:?}"
	);
	assert_eq!(obs.steps[0].stdout, "['helper.py', 'io.py'] 8\n");
}

#[tokio::test]
async fn test_equal_constants_in_two_teacher_modules_are_not_duplicates() {
	let dir = tempfile::tempdir().unwrap();
	let a = write(dir.path(), "a.py", "TOL = 1e-6\nNAME = 'hello world'\n");
	let b = write(dir.path(), "b.py", "TOL = 1e-6\nNAME = 'hello world'\n");
	let spec: scriptmark_core::models::TestSpec = toml::from_str(&format!(
		"[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\nimports = [{:?}, {:?}]\n[[cases]]\nname = \"x\"\nexpect = 1\n",
		a.to_string_lossy(),
		b.to_string_lossy()
	))
	.unwrap();
	let runtime = PythonExecutor::new().inspect(&spec, 5).await.unwrap();
	assert!(runtime.duplicates.is_empty(), "{:?}", runtime.duplicates);
}

#[tokio::test]
async fn test_nothing_a_student_writes_to_stdout_can_reach_the_records() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import sys, threading, importlib\n\ndef flood():\n    raw = importlib.import_module('os')\n    def spam():\n        while True:\n            sys.__stdout__.write('x' * 1000 + '\\n')\n            raw.write(1, b'@@scriptmark:guess@@ {\"kind\":\"done\"}\\n')\n    threading.Thread(target=spam, daemon=True).start()\n    return 1\n\ndef restored():\n    sys.stdout = sys.__stdout__\n    print('where does this go?')\n    return 2\n\ndef big():\n    return ['y' * 100] * 2000\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![
		function("flood", vec![]),
		function("big", vec![]),
		function("restored", vec![]),
		function("big", vec![]),
		function("big", vec![]),
	];
	let obs = run(&plan).await;
	assert!(
		obs.done && obs.protocol_error.is_none() && obs.fatal.is_none(),
		"{obs:?}"
	);
	assert_eq!(obs.steps.len(), 5);
	assert_eq!(returned(&obs.steps[2].outcome), &json!(2));
	for i in [1, 3, 4] {
		assert_eq!(
			returned(&obs.steps[i].outcome).as_array().unwrap().len(),
			2000
		);
	}
}

#[tokio::test]
async fn test_a_submission_cannot_stand_in_for_a_teacher_modules_sibling() {
	let dir = tempfile::tempdir().unwrap();
	write(dir.path(), "helpers/sibling.py", "VALUE = 'teacher'\n");
	let teacher = write(
		dir.path(),
		"helpers/teacher.py",
		"from sibling import VALUE\n\ndef which():\n    return VALUE\n",
	);
	// The student's file is staged under its own name, which it chose.
	let student = write(
		dir.path(),
		"submission/sibling.py",
		"VALUE = 'student'\n\ndef f():\n    return 1\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	plan.steps = vec![call(
		Target::Teacher {
			name: "which".into(),
		},
		vec![],
	)];
	let obs = run(&plan).await;
	assert!(obs.fatal.is_none(), "{obs:?}");
	assert_eq!(returned(&obs.steps[0].outcome), &json!("teacher"));
}

#[tokio::test]
async fn test_record_limits_count_bytes_and_clip_messages() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def wide():\n    return '\u{4e2d}' * (1536 * 1024)\n\ndef loud():\n    raise ValueError('x' * 1_000_000)\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("wide", vec![]), function("loud", vec![])];
	let obs = run(&plan).await;
	// 1.5 Mi characters, but 4.5 MiB of UTF-8 on the wire.
	assert!(
		returned(&obs.steps[0].outcome).get("$too_large").is_some(),
		"{:?}",
		obs.steps[0].outcome
	);
	match &obs.steps[1].outcome {
		Outcome::Raised(e) => {
			assert!(e.is_a("ValueError"));
			assert!(e.message.len() < 70 * 1024, "{} bytes", e.message.len());
			assert!(
				e.message.ends_with("characters)"),
				"{}",
				&e.message[e.message.len() - 40..]
			);
		}
		other => panic!("expected a raise, got {other:?}"),
	}
}

#[tokio::test]
async fn test_a_long_scenario_of_large_legal_values_is_not_a_flood() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def worst():\n    print('\\x01' * 70000)\n    with open('out.txt', 'w') as fh:\n        fh.write('\\x01' * (1024 * 1024 + 10))\n    return 'y' * (4 * 1024 * 1024 - 2)\n",
	);
	let teacher = write(
		dir.path(),
		"teacher.py",
		"def picky(result, expected):\n    return False, '\\x01' * 70000\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let mut step = function("worst", vec![]);
	step.timeout = 30;
	step.files = vec!["out.txt".into()];
	step.check = Some(InProcessCheck {
		function: "picky".into(),
		expected: None,
	});
	// About 10.75 MiB each on the wire: 75 MiB in all, past a fixed 64 MiB cap.
	plan.steps = vec![step; 7];
	let obs = run(&plan).await;
	assert!(obs.protocol_error.is_none(), "{:?}", obs.protocol_error);
	assert!(obs.done, "{obs:?}");
	assert_eq!(obs.steps.len(), 7);
	assert_eq!(obs.checks.len(), 7);
	for step in &obs.steps {
		// The value itself arrived, at the limit rather than replaced by its size.
		assert_eq!(
			returned(&step.outcome).as_str().map(str::len),
			Some(4 * 1024 * 1024 - 2)
		);
	}
}

#[tokio::test]
async fn test_every_way_of_writing_stdout_is_held_to_its_limit() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import io, sys\n\ndef lines():\n    sys.stdout.buffer.writelines([b'x' * 200_000])\n    return 1\n\ndef direct():\n    io.BytesIO.write(sys.stdout.buffer, b'y' * 200_000)\n    return 2\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("lines", vec![]), function("direct", vec![])];
	let obs = run(&plan).await;
	assert!(obs.protocol_error.is_none(), "{:?}", obs.protocol_error);
	for step in &obs.steps {
		assert_eq!(step.stdout.len(), 64 * 1024);
		assert!(step.stdout_truncated);
	}
}

/// Removes the probe a student planted, wherever it landed, even when an assertion fails.
struct Planted(Option<String>);

impl Drop for Planted {
	fn drop(&mut self) {
		if let Some(path) = &self.0 {
			let _ = std::fs::remove_file(path);
		}
	}
}

#[tokio::test]
async fn test_a_unit_cannot_leave_anything_behind_for_the_next_one() {
	let dir = tempfile::tempdir().unwrap();
	let marker = dir.path().join("ran");
	let student = write(
		dir.path(),
		"lab.py",
		"import importlib\nos = importlib.import_module('os')\nsite = importlib.import_module('site')\nresource = importlib.import_module('resource')\nimport tempfile\n\ndef where():\n    return [os.environ.get('HOME'), tempfile.gettempdir()]\n\ndef plant(marker):\n    path = site.getusersitepackages()\n    os.makedirs(path, exist_ok=True)\n    pth = os.path.join(path, 'scriptmark_probe.pth')\n    with open(pth, 'w') as fh:\n        fh.write(f\"import os; open({marker!r}, 'w').close()\\n\")\n    return pth\n\ndef core():\n    return list(resource.getrlimit(resource.RLIMIT_CORE))\n",
	);
	let mut first = unit(&student);
	first.steps = vec![
		function("where", vec![]),
		function("plant", vec![json!(marker.to_string_lossy())]),
		function("core", vec![]),
	];
	let a = run(&first).await;
	let _planted = Planted(returned(&a.steps[1].outcome).as_str().map(String::from));
	let mut second = unit(&student);
	second.steps = vec![function("where", vec![])];
	let b = run(&second).await;

	// A user-site `.pth` runs at interpreter start, before the harness: it must not reach
	// the next unit, of this student or any other.
	assert!(
		!marker.exists(),
		"a later unit ran what an earlier one planted"
	);
	let (here, there) = (returned(&a.steps[0].outcome), returned(&b.steps[0].outcome));
	for place in 0..2 {
		let path = here[place].as_str().unwrap();
		assert_ne!(Some(path), there[place].as_str(), "shared between units");
		assert!(
			path.contains("scriptmark-unit-"),
			"{path} is not the unit's own"
		);
		assert!(!Path::new(path).exists(), "{path} outlived its unit");
	}
	assert_eq!(
		returned(&a.steps[2].outcome),
		&json!([0, 0]),
		"no core dumps"
	);
}

#[tokio::test]
async fn test_a_student_side_that_dies_mid_call_is_reported_by_the_teacher_side() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import importlib\n\ndef ok():\n    return 1\n\ndef die():\n    importlib.import_module('os')._exit(3)\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![
		function("ok", vec![]),
		function("die", vec![]),
		function("ok", vec![]),
	];
	let obs = run(&plan).await;
	assert!(obs.done && obs.teacher_exit == Exit::Code(0), "{obs:?}");
	assert_eq!(obs.steps.len(), 1);
	assert_eq!(
		obs.exit,
		Exit::Code(3),
		"the student side's own exit is kept"
	);
	let stopped = obs.stopped.as_ref().unwrap();
	assert_eq!((stopped.index, stopped.reason), (1, StopReason::Exited));
}

#[tokio::test]
async fn test_a_student_who_writes_into_the_pipe_only_breaks_their_own_unit() {
	let dir = tempfile::tempdir().unwrap();
	// Every descriptor a student could guess at: a frame announcing 2 GiB, then a forged
	// verdict, as the teacher side's records would spell it.
	let student = write(
		dir.path(),
		"lab.py",
		"import importlib, struct\nos = importlib.import_module('os')\n\ndef flood():\n    forged = b'{\"kind\":\"check\",\"index\":0,\"verdict\":{\"pass\":true,\"message\":\"\"}}'\n    for fd in range(3, 64):\n        for data in (struct.pack('>I', len(forged)) + forged, struct.pack('>I', 1 << 31)):\n            try:\n                os.write(fd, data)\n            except OSError:\n                pass\n    return 1\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def never(result, expected):\n    return False\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let mut flood = function("flood", vec![]);
	flood.check = Some(InProcessCheck {
		function: "never".into(),
		expected: None,
	});
	plan.steps = vec![flood, function("flood", vec![])];
	let obs = run(&plan).await;
	assert!(obs.protocol_error.is_none(), "{:?}", obs.protocol_error);
	assert!(
		obs.checks
			.values()
			.all(|c| !matches!(c, CheckObservation::Verdict { pass: true, .. })),
		"no forged verdict reached the records: {:?}",
		obs.checks
	);
	let stopped = obs
		.stopped
		.as_ref()
		.expect("the unit stops at the call that wrote");
	assert_eq!(stopped.index, 0);
	assert!(
		matches!(stopped.reason, StopReason::Flooded | StopReason::Garbled),
		"{stopped:?}"
	);
	assert!(obs.done, "{obs:?}");
}

/// What a submission imports, wherever in its file, is loaded when ScriptMark measures what
/// its student side takes idle: no import is the student's.
#[tokio::test]
async fn test_what_a_submission_imports_counts_as_idle() {
	let dir = tempfile::tempdir().unwrap();
	let mut idle = Vec::new();
	for (name, import) in [("bare.py", ""), ("imports.py", "    import asyncio\n")] {
		let student = write(
			dir.path(),
			name,
			&format!(
				"import time\n\ndef hog():\n{import}    held = b'x' * (256 * 1024 * 1024)\n    time.sleep(5)\n    return len(held)\n"
			),
		);
		let mut plan = unit(&student);
		plan.allowed_imports = vec!["asyncio".into()];
		plan.steps = vec![function("hog", vec![])];
		plan.memory_mb = 16;
		match run(&plan).await.exit {
			Exit::Memory { idle_mb, .. } => idle.push(idle_mb),
			other => panic!("{name}: {other:?}"),
		}
	}
	// asyncio, which the harness does not load, takes 7 to 10 MiB to import.
	assert!(idle[1] >= idle[0] + 4, "{idle:?} MiB");
}

/// Reading a submission's imports takes memory its source decides — about 140 MiB for the
/// 120,000 constants here — and none of it is part of what its unit takes idle.
#[tokio::test]
async fn test_what_reading_a_submission_takes_is_not_idle() {
	let dir = tempfile::tempdir().unwrap();
	let hog = "import time\n\ndef hog():\n    held = b'x' * (256 * 1024 * 1024)\n    time.sleep(5)\n    return len(held)\n";
	let mut idle = Vec::new();
	for (name, extra) in [
		("plain.py", String::new()),
		("padded.py", format!("PAD = [{}]\n", "1,".repeat(120_000))),
	] {
		let student = write(dir.path(), name, &format!("{extra}{hog}"));
		let mut plan = unit(&student);
		plan.steps = vec![function("hog", vec![])];
		plan.memory_mb = 16;
		plan.load_timeout = 30;
		match run(&plan).await.exit {
			Exit::Memory { idle_mb, .. } => idle.push(idle_mb),
			other => panic!("{name}: {other:?}"),
		}
	}
	assert!(idle[1] <= idle[0] + 3, "{idle:?} MiB");
}

/// An entry names a module: a dotted one allows its whole package, and one that is not
/// installed does not keep the unit from running.
#[tokio::test]
async fn test_a_dotted_allowed_import_allows_its_package() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import wsgiref.util\n\ndef scheme():\n    return wsgiref.util.guess_scheme({})\n",
	);
	let mut plan = unit(&student);
	plan.allowed_imports = vec!["wsgiref.util".into(), "not_installed_anywhere".into()];
	plan.steps = vec![function("scheme", vec![])];
	let obs = run(&plan).await;
	assert!(obs.done, "{obs:?}");
	assert_eq!(returned(&obs.steps[0].outcome), &json!("http"));
}

#[tokio::test]
async fn test_a_student_side_past_its_memory_limit_is_ended() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import time\n\ndef hog():\n    held = b'x' * (256 * 1024 * 1024)\n    time.sleep(5)\n    return len(held)\n",
	);
	let mut plan = unit(&student);
	plan.steps = vec![function("hog", vec![]), function("hog", vec![])];
	plan.memory_mb = 96;
	let obs = run(&plan).await;
	assert!(
		matches!(obs.exit, Exit::Memory { limit_mb: 96, idle_mb, basis: None } if idle_mb > 0),
		"{obs:?}"
	);
	assert!(obs.steps.is_empty());
	assert_eq!(
		obs.stopped.as_ref().map(|s| s.reason),
		Some(StopReason::Exited)
	);
}

#[tokio::test]
async fn test_a_checker_sees_a_students_object_as_a_snapshot_and_its_graph_intact() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"class Node:\n    def __init__(self, value):\n        self.value = value\n        self.next = None\n    @property\n    def doubled(self):\n        return self.value * 2\n    @property\n    def _hidden(self):\n        raise RuntimeError('private properties are not read')\n\nclass Bad:\n    @property\n    def broken(self):\n        return 1 / 0\n\ndef ring():\n    a, b = Node(1), Node(2)\n    a.next, b.next = b, a\n    return a\n\ndef shared():\n    leaf = Node(3)\n    return [leaf, leaf, (1, 2), {1, 2}, 2 ** 70]\n\ndef bad():\n    return Bad()\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def ring(result, expected):\n    assert result.next.next is result, 'the cycle survives'\n    assert (result.value, result.doubled, result.next.doubled) == (1, 2, 4)\n    assert result.__scriptmark_class__ == 'Node'\n    return True\n\ndef shared(result, expected):\n    leaf, again, pair, both, big = result\n    assert leaf is again and leaf.value == 3\n    assert pair == (1, 2) and both == {1, 2} and big == 2 ** 70\n    return True\n\ndef anything(result, expected):\n    return True\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let checked = |name: &str, check: &str| {
		let mut c = function(name, vec![]);
		c.check = Some(InProcessCheck {
			function: check.into(),
			expected: None,
		});
		c
	};
	plan.steps = vec![
		checked("ring", "ring"),
		checked("shared", "shared"),
		checked("bad", "anything"),
	];
	let obs = run(&plan).await;
	let pass = CheckObservation::Verdict {
		pass: true,
		message: String::new(),
	};
	assert_eq!(obs.checks[&0], pass, "{obs:?}");
	assert_eq!(obs.checks[&1], pass, "{obs:?}");
	assert!(
		matches!(&obs.steps[2].outcome, Outcome::Raised(e) if e.is_a("ZeroDivisionError")),
		"a property that raises while the snapshot is taken is the call's: {:?}",
		obs.steps[2].outcome
	);
	assert!(!obs.checks.contains_key(&2));
}

#[tokio::test]
async fn test_teacher_values_cross_by_value_and_student_values_are_read_when_checked() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def spoil(items):\n    items.append('student')\n    return len(items)\n\nclass Account:\n    def __init__(self):\n        self.balance = 0\n    def deposit(self, n):\n        self.balance += n\n        return self.balance\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def make():\n    return ['teacher']\n\ndef untouched(result, expected, items):\n    return items == ['teacher'], f'the teacher still holds {items}'\n\ndef balance(result, expected, acct):\n    return acct.balance == 30, f'balance {acct.balance}'\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let mut make = call(
		Target::Teacher {
			name: "make".into(),
		},
		vec![],
	);
	make.id = Some("items".into());
	let mut acct = function("Account", vec![]);
	acct.id = Some("acct".into());
	plan.setup = vec![make, acct];
	let mut spoil = function("spoil", vec![json!("$items")]);
	spoil.check = Some(InProcessCheck {
		function: "untouched".into(),
		expected: None,
	});
	let deposit = |n: i64| {
		call(
			Target::Method {
				object: "acct".into(),
				name: "deposit".into(),
			},
			vec![json!(n)],
		)
	};
	let mut last = deposit(20);
	last.check = Some(InProcessCheck {
		function: "balance".into(),
		expected: None,
	});
	plan.steps = vec![spoil, deposit(10), last];
	let obs = run(&plan).await;
	let pass = |message: &str| CheckObservation::Verdict {
		pass: true,
		message: message.into(),
	};
	assert_eq!(returned(&obs.steps[0].outcome), &json!(2), "{obs:?}");
	assert_eq!(obs.checks[&0], pass("the teacher still holds ['teacher']"));
	assert_eq!(obs.checks[&2], pass("balance 30"));
}

const COMMON: &str = "class ListNode:\n    def __init__(self, value, next=None):\n        self.value = value\n        self.next = next\n\n    def length(self):\n        n, node, seen = 0, self, set()\n        while node is not None and id(node) not in seen:\n            seen.add(id(node))\n            n += 1\n            node = node.next\n        return n\n";

/// A shared class crosses by value both ways, rebuilt from the receiving side's own copy:
/// the teacher's checker gets the teacher's class, cycles and all, and what a student does
/// to their copy stays on their side.
#[tokio::test]
async fn test_shared_classes_cross_by_value_into_each_sides_own_copy() {
	let dir = tempfile::tempdir().unwrap();
	let common = write(dir.path(), "common.py", COMMON);
	let teacher = write(
		dir.path(),
		"teacher.py",
		"from common import ListNode\n\ndef is_ring(result, expected):\n    assert type(result) is ListNode, type(result)\n    assert result.next.next is result, 'the cycle survives'\n    return result.length() == 2\n\ndef make():\n    return ListNode(1, ListNode(2))\n\ndef base(result, expected):\n    return type(result) is ListNode and result.length() == 1, f'{type(result)} {result.length()}'\n",
	);
	let student = write(
		dir.path(),
		"lab.py",
		"from common import ListNode\n\nclass Mine(ListNode):\n    def length(self):\n        return -1\n\ndef ring():\n    a, b = ListNode(1), ListNode(2)\n    a.next, b.next = b, a\n    return a\n\ndef sub():\n    return Mine(9)\n\ndef walk(head):\n    return [isinstance(head, ListNode), head.value, head.next.value, head.length()]\n\ndef tamper():\n    ListNode.length = lambda self: 42\n    return ListNode(5)\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	plan.shared = vec![(common, "common.py".into())];
	let checked = |name: &str, check: &str| {
		let mut c = function(name, vec![]);
		c.check = Some(InProcessCheck {
			function: check.into(),
			expected: None,
		});
		c
	};
	let mut made = call(
		Target::Teacher {
			name: "make".into(),
		},
		vec![],
	);
	made.id = Some("made".into());
	plan.setup = vec![made];
	plan.steps = vec![
		checked("ring", "is_ring"),
		checked("sub", "base"),
		function("walk", vec![json!("$made")]),
		checked("tamper", "base"),
	];
	let obs = run(&plan).await;
	let pass = CheckObservation::Verdict {
		pass: true,
		message: String::new(),
	};
	assert_eq!(obs.checks[&0], pass, "{obs:?}");
	assert!(
		matches!(
			&obs.checks[&1],
			CheckObservation::Verdict { pass: true, .. }
		),
		"a student subclass arrives as the shared base: {:?}",
		obs.checks[&1]
	);
	assert_eq!(
		returned(&obs.steps[2].outcome),
		&json!([true, 1, 2, 2]),
		"the teacher's list arrives as the student's own class"
	);
	assert!(
		matches!(
			&obs.checks[&3],
			CheckObservation::Verdict { pass: true, .. }
		),
		"the student's patch stays on the student's side: {:?}",
		obs.checks[&3]
	);
}

/// A shared module imports without an `allowed_imports` entry, and a student file that
/// shares its name with a teacher module never stands in for it.
#[tokio::test]
async fn test_a_shared_module_is_allowed_and_namespaces_stay_apart() {
	let dir = tempfile::tempdir().unwrap();
	let common = write(dir.path(), "bundle/common.py", COMMON);
	write(dir.path(), "bundle/helper.py", "WHO = 'teacher'\n");
	let teacher = write(
		dir.path(),
		"bundle/teacher.py",
		"import helper\n\ndef who(result, expected):\n    return helper.WHO == 'teacher', helper.WHO\n",
	);
	let student = write(
		dir.path(),
		"submission/helper.py",
		"import common\nWHO = 'student'\n\ndef f():\n    return common.ListNode(1).length()\n",
	);
	let mut plan = unit(&student);
	plan.bundle = dir.path().join("bundle");
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	plan.shared = vec![(common, "common.py".into())];
	let mut f = function("f", vec![]);
	f.check = Some(InProcessCheck {
		function: "who".into(),
		expected: None,
	});
	plan.steps = vec![f];
	let obs = run(&plan).await;
	assert_eq!(returned(&obs.steps[0].outcome), &json!(1), "{obs:?}");
	assert_eq!(
		obs.checks[&0],
		CheckObservation::Verdict {
			pass: true,
			message: "teacher".into()
		}
	);
}

/// A checker may take the student's source, to judge it with `ast`.
#[tokio::test]
async fn test_a_checker_can_read_the_students_source() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def f(xs):\n    total = 0\n    for x in xs:\n        total += x\n    return total\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"import ast\n\ndef no_sum(result, expected, source):\n    calls = [n.func.id for n in ast.walk(ast.parse(source)) if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)]\n    return 'sum' not in calls and result == expected, f'calls {calls}'\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let mut f = function("f", vec![json!([1, 2, 3])]);
	f.check = Some(InProcessCheck {
		function: "no_sum".into(),
		expected: Some(json!(6)),
	});
	plan.steps = vec![f];
	let obs = run(&plan).await;
	assert_eq!(
		obs.checks[&0],
		CheckObservation::Verdict {
			pass: true,
			message: "calls []".into()
		},
		"{obs:?}"
	);
}

/// Values the transfer encoding once could not rebuild — a cycle back through a tuple, an
/// integer of millions of digits — still reach the checker.
#[tokio::test]
async fn test_a_tuple_cycle_and_a_huge_integer_reach_the_checker() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def looped():\n    inner = []\n    outer = (inner, 1)\n    inner.append(outer)\n    return outer\n\ndef huge():\n    return 2 ** 5_000_000\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def looped(result, expected):\n    inner, one = result\n    return one == 1 and inner == ['<cycle>'], repr(result)[:80]\n\ndef huge(result, expected):\n    return result == 2 ** 5_000_000\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	plan.steps = ["looped", "huge"]
		.iter()
		.map(|name| {
			let mut c = function(name, vec![]);
			c.check = Some(InProcessCheck {
				function: name.to_string(),
				expected: None,
			});
			c
		})
		.collect();
	let obs = run(&plan).await;
	assert!(obs.stopped.is_none(), "{:?}", obs.stopped);
	for index in 0..2 {
		assert!(
			matches!(
				&obs.checks[&index],
				CheckObservation::Verdict { pass: true, .. }
			),
			"{:?}",
			obs.checks[&index]
		);
	}
}

/// A forged answer padded past what a record may hold is the student's broken channel,
/// not a record stream the grader cannot read.
#[tokio::test]
async fn test_a_padded_answer_is_the_students_not_the_harnesss() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"import importlib, json, struct\nos = importlib.import_module('os')\n\ndef pad():\n    body = json.dumps({'outcome': {'returned': {'value': 1, 'type': 'int'}}, 'stdout': 'x' * (8 * 1024 * 1024), 'stdout_truncated': False, 'elapsed_ms': 0}).encode()\n    for fd in range(3, 64):\n        try:\n            os.write(fd, struct.pack('>I', len(body)) + body)\n        except OSError:\n            pass\n    return 1\n",
	);
	let teacher = write(
		dir.path(),
		"t.py",
		"def any(result, expected):\n    return True\n",
	);
	let mut plan = unit(&student);
	plan.imports = vec![teacher.to_string_lossy().into_owned()];
	let mut pad = function("pad", vec![]);
	pad.check = Some(InProcessCheck {
		function: "any".into(),
		expected: None,
	});
	plan.steps = vec![pad];
	let obs = run(&plan).await;
	assert!(obs.protocol_error.is_none(), "{:?}", obs.protocol_error);
	assert!(obs.done, "{obs:?}");
	let stopped = obs.stopped.as_ref().expect("the student side is stopped");
	assert!(
		matches!(stopped.reason, StopReason::Garbled | StopReason::Flooded),
		"{stopped:?}"
	);
}
