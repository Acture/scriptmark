//! The sandboxes, probed: what each side of a unit can reach. Every probe runs twice, the
//! second time without a sandbox, so a denial is the sandbox's doing and not a probe that
//! could never have worked.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark_core::models::Target;
use scriptmark_core::runner::executor::{
	CallPlan, Executor, Outcome, Subject, UnitObservation, UnitPlan,
};
use scriptmark_core::runner::python::PythonExecutor;
use scriptmark_core::runner::sandbox::Isolation;
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
	let path = dir.join(name);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(&path, content).unwrap();
	path
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

fn function(name: &str, args: Vec<Value>) -> CallPlan {
	call(
		Target::Function {
			name: name.to_string(),
		},
		args,
	)
}

fn teacher(name: &str, args: Vec<Value>) -> CallPlan {
	call(
		Target::Teacher {
			name: name.to_string(),
		},
		args,
	)
}

/// Every probe answers with what happened, rather than raising: `ok …` or the error's type.
const PROBES: &str = r#"import importlib
os = importlib.import_module('os')
socket = importlib.import_module('socket')
subprocess = importlib.import_module('subprocess')

def attempt(fn):
    try:
        return 'ok ' + str(fn())[:40]
    except BaseException as exc:
        return type(exc).__name__

def read(path):
    return attempt(lambda: open(path).read())

def listing(path):
    return attempt(lambda: sorted(os.listdir(path)))

def write(path):
    return attempt(lambda: open(path, 'w').write('x'))

def connect(port):
    return attempt(lambda: socket.create_connection(('127.0.0.1', port), timeout=3).close())

def spawn():
    return attempt(lambda: subprocess.run(['/bin/echo', 'escaped'], capture_output=True).stdout)

def run_program():
    # In place of this process: no fork, so no process limit stands in for the sandbox.
    return attempt(lambda: os.execv('/bin/echo', ['/bin/echo', 'escaped']))

def link_native():
    # The native-call module under a name of the student's own: its directory can be
    # listed, though the module itself cannot be read.
    sysconfig = importlib.import_module('sysconfig')
    where = os.path.join(sysconfig.get_paths()['platstdlib'], 'lib-dynload')
    name = next(n for n in os.listdir(where) if n.startswith('_ctypes'))
    return attempt(lambda: os.link(os.path.join(where, name), 'mine.so'))

def signal_grader():
    return attempt(lambda: os.kill(os.getppid(), 0))

def native():
    return attempt(lambda: importlib.import_module('ctypes').CDLL(None))

def environ_of_grader():
    return attempt(lambda: open(f'/proc/{os.getppid()}/environ', 'rb').read())

def env():
    return attempt(lambda: sorted(os.environ))

def chmod(path):
    stat = importlib.import_module('stat')
    def change():
        mode = os.stat(path).st_mode
        os.chmod(path, stat.S_IMODE(mode))  # the mode it already has: nothing to undo
    return attempt(change)

def unix_socket(path):
    return attempt(lambda: socket.socket(socket.AF_UNIX).connect(path))

def limits_of_grader():
    resource = importlib.import_module('resource')
    return attempt(lambda: resource.prlimit(os.getppid(), resource.RLIMIT_CPU))
"#;

struct Bench {
	_dir: tempfile::TempDir,
	root: PathBuf,
	bundle: PathBuf,
	answers: PathBuf,
	other: PathBuf,
	student: PathBuf,
	teacher: PathBuf,
	/// A port the grader listens on, which no unit may connect to.
	port: u16,
	_listener: std::net::TcpListener,
	/// A Unix socket the grader listens on, likewise.
	socket: PathBuf,
	_unix: std::os::unix::net::UnixListener,
}

impl Bench {
	fn new() -> Self {
		let dir = tempfile::tempdir().unwrap();
		let root = std::fs::canonicalize(dir.path()).unwrap();
		let bundle = root.join("tests");
		let answers = write(&bundle, "answers.txt", "the teacher's answers\n");
		let teacher = write(&bundle, "helpers/probes.py", PROBES);
		let other = write(&root, "submissions/bob/lab.py", "SECRET = 'bob'\n");
		let student = write(&root, "submissions/alice/lab.py", PROBES);
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		// Short: a socket path has a length limit, and a temporary directory can be deep.
		let unique = dir
			.path()
			.file_name()
			.unwrap()
			.to_string_lossy()
			.into_owned();
		let socket = std::env::temp_dir().join(format!("sm{unique}.sock"));
		let _ = std::fs::remove_file(&socket);
		let unix = std::os::unix::net::UnixListener::bind(&socket).unwrap();
		Self {
			socket,
			_unix: unix,
			port: listener.local_addr().unwrap().port(),
			_listener: listener,
			_dir: dir,
			root,
			bundle,
			answers,
			other,
			student,
			teacher,
		}
	}

	fn plan(&self, steps: Vec<CallPlan>) -> UnitPlan {
		UnitPlan {
			subject: Subject::Student,
			functions: scriptmark_core::matching::Functions::default(),
			file: self.student.clone(),
			script: None,
			imports: vec![self.teacher.to_string_lossy().into_owned()],
			vars: Arc::new(BTreeMap::new()),
			data_files: Vec::new(),
			bundle: self.bundle.clone(),
			shared: Vec::new(),
			memory_mb: scriptmark_core::spec_loader::DEFAULT_MEMORY_MB,
			allowed_imports: Vec::new(),
			load_timeout: 5,
			setup: Vec::new(),
			steps,
		}
	}
}

impl Drop for Bench {
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.socket);
	}
}

async fn both(plan: &UnitPlan) -> (UnitObservation, UnitObservation) {
	let sandboxed = PythonExecutor::new().run(plan).await;
	let open = PythonExecutor::new()
		.with_isolation(Isolation::Unisolated)
		.run(plan)
		.await;
	(sandboxed, open)
}

fn said(obs: &UnitObservation, index: usize) -> String {
	match &obs.steps[index].outcome {
		Outcome::Returned { value, .. } => value.as_str().unwrap_or_default().to_string(),
		other => panic!("step {index}: {other:?}\n{obs:?}"),
	}
}

/// What each probe said with the sandbox and without it, by name.
fn compare(
	names: &[&str],
	sandboxed: &UnitObservation,
	open: &UnitObservation,
) -> Vec<(String, String, String)> {
	assert!(sandboxed.done && sandboxed.sandboxed, "{sandboxed:?}");
	assert!(open.done && !open.sandboxed, "{open:?}");
	names
		.iter()
		.enumerate()
		.map(|(i, name)| (name.to_string(), said(sandboxed, i), said(open, i)))
		.collect()
}

fn denied(probes: &[(String, String, String)], name: &str) {
	let (_, sandboxed, open) = probes.iter().find(|(n, ..)| n == name).unwrap();
	if name == "spawn" {
		// The process limit is per user on macOS, where a busy machine already holds more
		// than a unit may start: fork can fail for that before the sandbox is asked.
		// `run program` probes the sandbox itself.
		assert!(!sandboxed.starts_with("ok"), "{name}: {sandboxed}");
		return;
	}
	if name == "run program" || name == "native by another name" {
		// Unsandboxed, the first replaces the process and never answers, and the second
		// depends on where the interpreter lives: no control to compare.
		assert_eq!(sandboxed, "PermissionError", "{name}: {sandboxed}");
		return;
	}
	assert!(
		open.starts_with("ok"),
		"{name}: the probe must work unsandboxed, said {open}"
	);
	assert!(
		!sandboxed.starts_with("ok"),
		"{name}: the sandbox let it through: {sandboxed}"
	);
}

fn allowed(probes: &[(String, String, String)], name: &str) {
	let (_, sandboxed, _) = probes.iter().find(|(n, ..)| n == name).unwrap();
	assert!(
		sandboxed.starts_with("ok"),
		"{name}: the sandbox refused it: {sandboxed}"
	);
}

#[tokio::test]
async fn test_the_student_side_reaches_only_its_own_directory() {
	let bench = Bench::new();
	let home = std::env::var("HOME").unwrap();
	let path = |p: &Path| json!(p.to_string_lossy());
	let mut probes = vec![
		("bundle", function("read", vec![path(&bench.answers)])),
		(
			"bundle listing",
			function("listing", vec![path(&bench.bundle)]),
		),
		(
			"teacher module",
			function("read", vec![path(&bench.teacher)]),
		),
		("other student", function("read", vec![path(&bench.other)])),
		(
			"submissions",
			function("listing", vec![path(&bench.root.join("submissions"))]),
		),
		("home", function("listing", vec![json!(home)])),
		(
			"write beside the bundle",
			function("write", vec![path(&bench.bundle.join("planted.py"))]),
		),
		("network", function("connect", vec![json!(bench.port)])),
		("spawn", function("spawn", vec![])),
		("signal", function("signal_grader", vec![])),
		("own directory", function("write", vec![json!("mine.txt")])),
		(
			"mode of a bundle file",
			function("chmod", vec![path(&bench.answers)]),
		),
		(
			"unix socket",
			function("unix_socket", vec![path(&bench.socket)]),
		),
	];
	let mut sandboxed_only = vec![("run program", function("run_program", vec![]))];
	if cfg!(target_os = "macos") {
		sandboxed_only.push(("native by another name", function("link_native", vec![])));
	}
	if cfg!(target_os = "macos") {
		// Seatbelt does not mediate reading another process's arguments and environment;
		// refusing native calls is what keeps the grader's out of reach.
		probes.push(("native calls", function("native", vec![])));
	}
	if cfg!(target_os = "linux") {
		probes.push((
			"grader's environment",
			function("environ_of_grader", vec![]),
		));
	}
	let names: Vec<&str> = probes.iter().map(|(n, _)| *n).collect();
	let plan = bench.plan(probes.into_iter().map(|(_, c)| c).collect());
	let (sandboxed, open) = both(&plan).await;
	let results = compare(&names, &sandboxed, &open);
	for name in names.iter().filter(|n| **n != "own directory") {
		denied(&results, name);
	}
	allowed(&results, "own directory");

	for (name, probe) in sandboxed_only {
		let obs = PythonExecutor::new().run(&bench.plan(vec![probe])).await;
		denied(&[(name.to_string(), said(&obs, 0), String::new())], name);
	}
}

#[tokio::test]
async fn test_the_teacher_side_reads_the_bundle_and_the_source_and_nothing_else() {
	let bench = Bench::new();
	let home = std::env::var("HOME").unwrap();
	let path = |p: &Path| json!(p.to_string_lossy());
	let probes = vec![
		("bundle", teacher("read", vec![path(&bench.answers)])),
		("other student", teacher("read", vec![path(&bench.other)])),
		(
			"submissions",
			teacher("listing", vec![path(&bench.root.join("submissions"))]),
		),
		("home", teacher("listing", vec![json!(home)])),
		(
			"write the bundle",
			teacher("write", vec![path(&bench.bundle.join("planted.py"))]),
		),
		("network", teacher("connect", vec![json!(bench.port)])),
		("spawn", teacher("spawn", vec![])),
		("signal", teacher("signal_grader", vec![])),
	];
	let exec = teacher("run_program", vec![]);
	let names: Vec<&str> = probes.iter().map(|(n, _)| *n).collect();
	let plan = bench.plan(probes.into_iter().map(|(_, c)| c).collect());
	let (sandboxed, open) = both(&plan).await;
	let results = compare(&names, &sandboxed, &open);
	allowed(&results, "bundle");
	for name in names.iter().filter(|n| **n != "bundle") {
		denied(&results, name);
	}
	let obs = PythonExecutor::new().run(&bench.plan(vec![exec])).await;
	denied(
		&[("run program".into(), said(&obs, 0), String::new())],
		"run program",
	);
}

#[tokio::test]
async fn test_neither_side_sees_the_graders_environment() {
	let bench = Bench::new();
	let plan = bench.plan(vec![function("env", vec![]), teacher("env", vec![])]);
	let obs = PythonExecutor::new().run(&plan).await;
	for index in 0..2 {
		let seen = said(&obs, index);
		assert!(
			!seen.contains("CANVAS") && !seen.contains("CARGO"),
			"step {index} saw {seen}"
		);
	}
}

/// A lint tool runs in the teacher's sandbox: it reads the submission's copy and the bundle,
/// and not another student's work.
#[test]
fn test_a_lint_tool_reads_the_bundle_and_the_copy_and_nothing_else() {
	use scriptmark_core::models::LintConfig;
	use scriptmark_core::runner::linter::run_lint;
	let bench = Bench::new();
	let reading = |path: &Path| LintConfig {
		// One finding line, if both reads succeed; a failed read exits 1, outside [0].
		command: format!(
			"sh -c 'cat \"$1\" > /dev/null && cat \"$2\" > /dev/null && echo finding' sh {{file}} {}",
			path.display()
		),
		max_warnings: 10,
		ok_exit_codes: vec![0],
	};
	let lint = |config: &LintConfig, isolation| {
		run_lint(
			config,
			&bench.student,
			&bench.bundle,
			isolation,
			scriptmark_core::runner::linter::LINT_TIMEOUT,
		)
	};
	assert_eq!(
		lint(&reading(&bench.answers), Isolation::Sandboxed),
		Ok(90.0)
	);
	assert_eq!(
		lint(&reading(&bench.other), Isolation::Unisolated),
		Ok(90.0)
	);
	let refused = lint(&reading(&bench.other), Isolation::Sandboxed).unwrap_err();
	assert!(refused.contains("exited with"), "{refused}");
}

/// A Python checker script runs in the teacher's sandbox too, with a cleared environment.
#[test]
fn test_a_python_checker_script_reads_the_bundle_and_nothing_else() {
	use scriptmark_core::checker::python_checker::PythonChecker;
	use scriptmark_core::checker::{CheckInput, Checker};
	let bench = Bench::new();
	let script = write(
		&bench.bundle,
		"verify.py",
		&format!(
			"import json, os, sys\njson.load(sys.stdin)\ndef can(path):\n    try:\n        open(path).read()\n        return 'yes'\n    except OSError:\n        return 'no'\nbundle, other = can({:?}), can({:?})\nprint(json.dumps({{'pass': True, 'message': f'bundle={{bundle}} other={{other}} env={{sorted(os.environ)}}'}}))\n",
			bench.answers, bench.other
		),
	);
	let input = CheckInput {
		result: json!(1),
		expected: json!(1),
		context: json!({}),
	};
	let message = |checker: PythonChecker| checker.check(&input).unwrap().message;
	let python = PythonExecutor::new().python_cmd().to_string();
	let open = message(PythonChecker::new(&script).with_python_cmd(&python));
	let confined = message(
		PythonChecker::new(&script)
			.with_python_cmd(&python)
			.confined(vec![bench.bundle.clone()]),
	);
	assert!(open.starts_with("bundle=yes other=yes"), "{open}");
	assert!(confined.starts_with("bundle=yes other=no"), "{confined}");
	assert!(
		!confined.contains("CARGO"),
		"the environment is cleared: {confined}"
	);
}
