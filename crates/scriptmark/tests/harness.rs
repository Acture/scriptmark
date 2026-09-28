//! The harness, observed directly through `Executor::run`: what a unit reports, before
//! anything is judged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::models::Target;
use scriptmark::runner::executor::{
	CallPlan, CheckObservation, Executor, Exit, InProcessCheck, Outcome, ScriptRun, Subject,
	UnitObservation, UnitPlan,
};
use scriptmark::runner::python::PythonExecutor;
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
		file: file.to_path_buf(),
		script: None,
		imports: Vec::new(),
		vars: Arc::new(BTreeMap::new()),
		data_files: Vec::new(),
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

#[tokio::test]
async fn test_a_unit_that_never_yields_is_killed_at_its_deadline() {
	let dir = tempfile::tempdir().unwrap();
	let student = write(
		dir.path(),
		"lab.py",
		"def forever():\n    while True:\n        try:\n            while True:\n                pass\n        except BaseException:\n            pass\n\ndef ok():\n    return 1\n",
	);
	let mut plan = unit(&student);
	plan.load_timeout = 1;
	let mut forever = function("forever", vec![]);
	forever.timeout = 1;
	let mut ok = function("ok", vec![]);
	ok.timeout = 1;
	plan.steps = vec![forever, ok];
	let obs = run(&plan).await;
	assert_eq!(obs.exit, Exit::Deadline);
	assert!(obs.steps.is_empty(), "the hung call never reported");
	assert!(!obs.done);
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
	let spec: scriptmark::models::TestSpec = toml::from_str(&format!(
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
	let spec_of = |imports: &[&PathBuf]| -> scriptmark::models::TestSpec {
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
	let spec: scriptmark::models::TestSpec = toml::from_str(&format!(
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
