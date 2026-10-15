//! The Python backend. A unit runs as two processes joined by a pair of pipes: the teacher
//! side (`harness/teacher.py`) holds the teacher's modules and checkers, drives the plan
//! and writes the records; the student side (`harness/student.py`) holds only the
//! student's module. ScriptMark starts both, connects them to each other, and reads the
//! teacher side's records; it never relays what passes between them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::process::{Child, Command};

use crate::models::TestSpec;
use crate::runner::executor::{
	Executor, Exit, GRACE_SECS, STARTUP_SECS, Subject, TeacherRuntime, UnitObservation, UnitPlan,
};
use crate::runner::records::{Bounds, Reader, Records};
#[cfg(unix)]
use crate::runner::sandbox::apply_sandbox;
use crate::runner::sandbox::{Confinement, Isolation, covering};

/// Resolve a python command to an absolute path so env_clear() doesn't change
/// which interpreter runs. Falls back to the original string if resolution fails.
fn resolve_python_path(cmd: &str) -> String {
	let path = Path::new(cmd);
	if path.is_absolute() {
		return cmd.to_string();
	}
	// A relative path, `.venv/bin/python`, is the grader's: each side starts elsewhere.
	if path.components().count() > 1 {
		return std::path::absolute(path)
			.map(|p| p.to_string_lossy().into_owned())
			.unwrap_or_else(|_| cmd.to_string());
	}
	if let Ok(output) = std::process::Command::new("which").arg(cmd).output()
		&& output.status.success()
		&& let Ok(path) = String::from_utf8(output.stdout)
	{
		let path = path.trim();
		if !path.is_empty() {
			return path.to_string();
		}
	}
	cmd.to_string()
}

/// Executor for Python student code.
pub struct PythonExecutor {
	python_cmd: String,
	/// The most memory the student side may hold before it is ended, in MiB.
	isolation: Isolation,
}

impl PythonExecutor {
	pub fn new() -> Self {
		Self::with_python_cmd("python3")
	}

	pub fn with_python_cmd(python_cmd: impl Into<String>) -> Self {
		Self {
			python_cmd: resolve_python_path(&python_cmd.into()),
			isolation: Isolation::default(),
		}
	}

	/// Whether each side runs in its own sandbox. Sandboxed is the default; a unit whose
	/// sandbox cannot be applied is not graded.
	pub fn with_isolation(mut self, isolation: Isolation) -> Self {
		self.isolation = isolation;
		self
	}

	pub fn isolation(&self) -> Isolation {
		self.isolation
	}

	pub fn python_cmd(&self) -> &str {
		&self.python_cmd
	}
}

/// What each interpreter needs to run, found once per interpreter, the first time
/// anything it runs is confined.
static INTERPRETERS: LazyLock<
	Mutex<std::collections::HashMap<String, Result<Interpreter, String>>>,
> = LazyLock::new(Default::default);

fn interpreter(python_cmd: &str) -> Result<Interpreter, String> {
	INTERPRETERS
		.lock()
		.expect("interpreter cache poisoned")
		.entry(python_cmd.to_string())
		.or_insert_with(|| Interpreter::probe(python_cmd))
		.clone()
}

/// A confinement for teacher code `python_cmd` runs outside a unit — a Python checker
/// script, or a lint tool written in Python: the interpreter, `read`, and `write`.
pub fn teacher_confinement(
	python_cmd: &str,
	read: Vec<PathBuf>,
	write: Vec<PathBuf>,
	spawn: bool,
) -> Result<Confinement, String> {
	let interpreter = interpreter(python_cmd)?;
	Ok(Confinement {
		read: covering(
			interpreter
				.read
				.into_iter()
				.chain(canonical(read))
				.collect(),
		),
		write,
		exec: interpreter.exec,
		spawn,
		no_ffi: !spawn,
	})
}

/// What one Python interpreter reads and runs: its prefixes, its standard library and
/// packages, and the shared libraries they load — not the rest of the machine.
#[derive(Debug, Clone)]
struct Interpreter {
	read: Vec<PathBuf>,
	exec: Vec<PathBuf>,
}

/// The oldest Python the harness runs on.
const MINIMUM_PYTHON: (u32, u32) = (3, 10);

/// Asks the interpreter where it lives, unconfined and once: its prefixes, and where its
/// standard library and installed packages are — not every `sys.path` entry, which a
/// package's `.pth` file can point anywhere, a home directory included.
const PROBE: &str = "import json, os, sys, sysconfig
places = sysconfig.get_paths()
paths = [sys.prefix, sys.base_prefix, sys.exec_prefix, sys.base_exec_prefix, *(places[k] for k in ('stdlib', 'platstdlib', 'purelib', 'platlib'))]
print(json.dumps({
	'version': list(sys.version_info[:2]),
	'executable': os.path.realpath(sys.executable),
	'base': os.path.realpath(sys.base_prefix),
	'paths': [os.path.realpath(p) for p in paths if p and os.path.exists(p)],
}))";

impl Interpreter {
	fn probe(python_cmd: &str) -> Result<Self, String> {
		#[derive(serde::Deserialize)]
		struct Found {
			version: (u32, u32),
			executable: PathBuf,
			base: PathBuf,
			paths: Vec<PathBuf>,
		}
		let output = std::process::Command::new(python_cmd)
			.env_clear()
			.args(["-I", "-c", PROBE])
			.output()
			.map_err(|e| format!("could not ask {python_cmd} where it lives: {e}"))?;
		let found: Found = serde_json::from_slice(&output.stdout).map_err(|e| {
			format!(
				"{python_cmd} did not say where it lives ({e}): {}",
				String::from_utf8_lossy(&output.stderr).trim()
			)
		})?;
		// The harness uses `X | None` and `match`-era typing: older interpreters die starting it.
		if found.version < MINIMUM_PYTHON {
			return Err(format!(
				"{python_cmd} is Python {}.{}; ScriptMark needs Python {}.{} or newer",
				found.version.0, found.version.1, MINIMUM_PYTHON.0, MINIMUM_PYTHON.1
			));
		}
		let command = PathBuf::from(python_cmd);
		let mut read = found.paths;
		read.extend(system_libraries(&found.base));
		// The command itself may be a link into the interpreter's prefix, as a venv's is.
		read.push(command.clone());
		let mut exec = vec![command];
		#[cfg(target_os = "macos")]
		// A framework build's `python3` starts `Python.app` inside its prefix.
		exec.push(found.base);
		#[cfg(not(target_os = "macos"))]
		{
			exec.extend(crate::runner::sandbox::loader_of(&found.executable));
			let _ = found.base;
		}
		exec.push(found.executable);
		Ok(Self {
			read: covering(read),
			exec,
		})
	}
}

/// Shared libraries an interpreter loads from outside its prefix.
#[cfg(target_os = "macos")]
fn system_libraries(base: &Path) -> Vec<PathBuf> {
	// A Homebrew keg links against other kegs through `opt`: those, and only those — not
	// the prefix's `etc` or `var`, which hold service configuration and data.
	let text = base.to_string_lossy();
	match text.find("/Cellar/") {
		Some(at) => {
			let prefix = Path::new(&text[..at]);
			vec![prefix.join("Cellar"), prefix.join("opt")]
		}
		None => Vec::new(),
	}
}

#[cfg(not(target_os = "macos"))]
fn system_libraries(_base: &Path) -> Vec<PathBuf> {
	[
		"/usr/lib",
		"/usr/lib64",
		"/lib",
		"/lib64",
		"/usr/local/lib",
		"/etc/ld.so.cache",
		"/etc/localtime",
		"/usr/share/zoneinfo",
	]
	.iter()
	.filter_map(|p| std::fs::canonicalize(p).ok())
	.collect()
}

const COMMON: &str = include_str!("harness/common.py");
const TEACHER: &str = include_str!("harness/teacher.py");
const STUDENT: &str = include_str!("harness/student.py");
const MATCHING: &str = include_str!("../matching.py");

/// Every line either side runs, for the frozen contract: a change to any of it can change
/// what a reference implementation is observed to do.
pub(crate) const PROTOCOL: [&str; 4] = [COMMON, TEACHER, STUDENT, MATCHING];

/// `python -c` puts the working directory first on `sys.path`: it goes before any import.
const PRELUDE: &str = "import sys; sys.path[:] = [p for p in sys.path if p]\n";
static TEACHER_SCRIPT: LazyLock<String> = LazyLock::new(|| format!("{PRELUDE}{COMMON}\n{TEACHER}"));
static STUDENT_SCRIPT: LazyLock<String> =
	LazyLock::new(|| format!("{PRELUDE}{MATCHING}\n{COMMON}\n{STUDENT}"));

/// What one call's record may carry, which both sides are given rather than assume.
#[derive(Debug, Clone, Copy, Serialize)]
struct Limits {
	stdout: usize,
	file: usize,
	value: usize,
	message: usize,
	/// The most a value handed to the other side's code may take, as JSON.
	transfer: usize,
	depth: usize,
}

const LIMITS: Limits = Limits {
	stdout: 64 << 10,
	file: 1 << 20,
	value: 4 << 20,
	message: 64 << 10,
	transfer: 32 << 20,
	depth: 100,
};

/// The most a record carries besides its value, stdout, message and files: its target,
/// the matching evidence, and the keys around them.
const SLACK: usize = 1 << 20;

/// The tail of stderr kept for diagnosing a crash.
const STDERR_TAIL: usize = 4096;

/// How long the student side has to finish by itself once the teacher side has: it was
/// told to, and only a hung or stopped student side takes longer.
const LINGER: Duration = Duration::from_millis(500);

/// The descriptor the teacher side writes records to. On unix it is a pipe of its own;
/// elsewhere stderr, since stdin and stdout are its pipe to the student side.
#[cfg(unix)]
const RECORD_FD: i32 = 3;
#[cfg(not(unix))]
const RECORD_FD: i32 = 2;

/// What the teacher side's record stream may hold for `plan`: its largest record — a call
/// with its value, stdout, an error message and every file it observes, each of which JSON
/// escaping can grow sixfold — and one record per call and check, plus the frame around
/// them.
fn record_bounds(plan: &UnitPlan) -> Bounds {
	let files = plan
		.setup
		.iter()
		.chain(&plan.steps)
		.map(|c| c.files.len())
		.chain(plan.script.as_ref().map(|s| s.files.len()))
		.max()
		.unwrap_or(0);
	let per_file = 6 * LIMITS.file + 4096;
	Bounds {
		frame: (SLACK + LIMITS.value + 6 * (LIMITS.stdout + LIMITS.message))
			.saturating_add(files.saturating_mul(per_file)),
		// ready, load, done, and a fatal or a stop, besides the calls and checks.
		count: 5 + plan.setup.len() + 2 * plan.steps.len().max(1),
	}
}

/// What an inspection may report: every export and its parameters.
const INSPECT_BOUNDS: Bounds = Bounds {
	frame: 64 << 20,
	count: 4,
};

/// One side's private directories.
struct Side {
	work: PathBuf,
	/// Its own `HOME` and `TMPDIR`, so nothing one unit leaves there reaches another.
	home: PathBuf,
	tmp: PathBuf,
}

/// A unit's private directory: one working directory per side, the teacher side's
/// payload, and a read-only copy of the subject's source.
struct Staged {
	/// Removed on drop.
	root: tempfile::TempDir,
	teacher: Side,
	student: Side,
	payload: PathBuf,
	/// The subject file's copy in the student side's working directory.
	subject: Option<PathBuf>,
	/// The subject's source, for teacher code to read.
	source: Option<PathBuf>,
	/// A file outside everything either side may read: a side that can open it is not in
	/// its sandbox, and refuses to go on.
	canary: PathBuf,
}

/// Copy `src` (a file or a directory) to `dest`, creating parents.
fn copy_into(src: &Path, dest: &Path) -> std::io::Result<()> {
	if let Some(parent) = dest.parent() {
		std::fs::create_dir_all(parent)?;
	}
	if src.is_dir() {
		std::fs::create_dir_all(dest)?;
		for entry in std::fs::read_dir(src)? {
			let entry = entry?;
			copy_into(&entry.path(), &dest.join(entry.file_name()))?;
		}
		Ok(())
	} else {
		std::fs::copy(src, dest).map(|_| ())
	}
}

fn side(root: &Path, name: &str) -> std::io::Result<Side> {
	let side = Side {
		work: root.join(name),
		home: root.join(format!("{name}-home")),
		tmp: root.join(format!("{name}-tmp")),
	};
	for dir in [&side.work, &side.home, &side.tmp] {
		std::fs::create_dir(dir)?;
	}
	Ok(side)
}

fn stage(
	subject: Option<&Path>,
	data_files: &[(PathBuf, String)],
	shared: &[(PathBuf, String)],
	payload: impl FnOnce(&Staged) -> serde_json::Value,
) -> std::io::Result<Staged> {
	let root = tempfile::Builder::new()
		.prefix("scriptmark-unit-")
		.tempdir()?;
	// The sandbox sees resolved paths: `/var/folders/…` is `/private/var/folders/…` on macOS.
	let base = std::fs::canonicalize(root.path())?;
	let teacher = side(&base, "teacher")?;
	let student = side(&base, "student")?;
	let canary = base.join("canary");
	std::fs::write(&canary, "a sandboxed side must not be able to read this\n")?;
	// Teacher code that opens a data file by its relative path finds it, as student code does.
	for (src, rel) in data_files {
		copy_into(src, &student.work.join(rel))?;
		copy_into(src, &teacher.work.join(rel))?;
	}
	// The student side's own copy of each shared module; the teacher side's is the bundle's.
	for (src, rel) in shared {
		copy_into(src, &student.work.join(rel))?;
	}
	let (copied, source) = match subject {
		Some(file) => {
			let name = file
				.file_name()
				.unwrap_or(std::ffi::OsStr::new("subject.py"));
			let dest = student.work.join(name);
			std::fs::copy(file, &dest)?;
			let sources = base.join("source");
			std::fs::create_dir(&sources)?;
			let source = sources.join(name);
			std::fs::copy(file, &source)?;
			let mut readonly = std::fs::metadata(&source)?.permissions();
			readonly.set_readonly(true);
			std::fs::set_permissions(&source, readonly)?;
			(Some(dest), Some(source))
		}
		None => (None, None),
	};
	let staged = Staged {
		// The teacher side's argument: it reads the payload and removes it.
		payload: teacher.work.join("payload.json"),
		teacher,
		student,
		subject: copied,
		source,
		canary,
		root,
	};
	std::fs::write(&staged.payload, payload(&staged).to_string())?;
	Ok(staged)
}

/// What a finished unit left behind.
struct Finished {
	records: Records,
	exit: Exit,
	teacher_exit: Exit,
	stderr: String,
	/// The most the student side was seen holding, in bytes.
	peak: u64,
}

impl Finished {
	fn spawn_failed(message: String) -> Self {
		Self {
			records: Records::default(),
			exit: Exit::Spawn(message.clone()),
			teacher_exit: Exit::Spawn(message),
			stderr: String::new(),
			peak: 0,
		}
	}
}

/// What the student side may hold: `limit_mb` beyond `idle`, the bytes it takes idle,
/// sampled `every` so often.
#[derive(Debug, Clone, Copy)]
struct Memory {
	limit_mb: u64,
	idle: u64,
	every: Duration,
}

/// How often a unit's student side is sampled.
const SAMPLE_EVERY: Duration = Duration::from_millis(50);

impl Memory {
	fn watched(self) -> u64 {
		self.idle
			.saturating_add(self.limit_mb.saturating_mul(1 << 20))
	}

	fn total_mb(self) -> u64 {
		self.watched().div_ceil(1 << 20)
	}

	fn exceeded(self) -> Exit {
		Exit::Memory {
			limit_mb: self.limit_mb,
			idle_mb: self.idle.div_ceil(1 << 20),
		}
	}
}

/// Values worked out once per key, by a digest of everything they depend on. A failure is
/// not kept: the next caller works it out again.
#[cfg(any(target_os = "macos", target_os = "linux"))]
type Once<T> = LazyLock<Mutex<std::collections::HashMap<String, Arc<tokio::sync::OnceCell<T>>>>>;

/// The modules each submission imports, by its source.
#[cfg(any(target_os = "macos", target_os = "linux"))]
static SOURCE_IMPORTS: Once<Imports> = LazyLock::new(Default::default);

/// What the student side takes idle, by what it imports and what the teacher gives it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
static IDLE_MEMORY: Once<u64> = LazyLock::new(Default::default);

/// Modules, each with what `from … import` takes from it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
type Imports = Vec<(String, Vec<String>)>;

/// The student whose idle memory is measured (`harness/idle.py`).
#[cfg(any(target_os = "macos", target_os = "linux"))]
const IDLE: &str = include_str!("harness/idle.py");

/// Reads a source's imports without running it (`harness/imports.py`).
#[cfg(any(target_os = "macos", target_os = "linux"))]
const IMPORTS: &str = include_str!("harness/imports.py");

/// The largest source whose imports are read beforehand: a larger one's count. Parsing
/// takes several hundred times the source (500 KB of constants: about 290 MiB), and the
/// reader is not watched; a submission is a few kilobytes.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const IMPORTS_SOURCE_LIMIT: usize = 256 << 10;

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn digest(given: &serde_json::Value) -> String {
	use sha2::{Digest, Sha256};
	format!("{:x}", Sha256::digest(given.to_string()))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn once<T>(cache: &Once<T>, key: String) -> Arc<tokio::sync::OnceCell<T>> {
	cache
		.lock()
		.expect("once cache poisoned")
		.entry(key)
		.or_default()
		.clone()
}

impl PythonExecutor {
	/// A command for one side: confined when sandboxed, with a cleared environment, the
	/// side's own `HOME` and `TMPDIR`, and the kernel's limits.
	fn command(
		&self,
		script: &str,
		side: &Side,
		deadline_secs: u64,
		memory_mb: u64,
		confinement: Option<&Confinement>,
	) -> Result<Command, String> {
		// `memory_mb`: all the side may hold, interpreter included.
		let mut cmd = match confinement {
			Some(c) => Command::from(
				crate::runner::sandbox::confined(Path::new(&self.python_cmd), c)
					.map_err(|e| format!("cannot sandbox this unit: {e}"))?,
			),
			None => Command::new(&self.python_cmd),
		};
		cmd.env_clear()
			.env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
			.env("HOME", &side.home)
			.env("TMPDIR", &side.tmp)
			// Isolated: no user site-packages, whose `.pth` files run before the harness, no
			// PYTHON* variables, and the working directory off sys.path. What those variables
			// used to set is spelled as flags.
			.args(["-I", "-B", "-X", "utf8", "-c", script])
			.current_dir(&side.work)
			.kill_on_drop(true);
		#[cfg(windows)]
		if let Some(root) = std::env::var_os("SYSTEMROOT") {
			// Python cannot seed its hash randomisation on Windows without it.
			cmd.env("SYSTEMROOT", root);
		}
		#[cfg(unix)]
		{
			// Its own process group, so anything it spawns dies with it.
			cmd.process_group(0);
			apply_sandbox(
				&mut cmd,
				&sandbox_for(deadline_secs, memory_mb, confinement.is_some()),
			);
		}
		#[cfg(not(unix))]
		let _ = (deadline_secs, memory_mb);
		Ok(cmd)
	}

	/// Each side's confinement, when sandboxed: the interpreter, its own directories, and
	/// what `teacher` and `student` add. Neither side makes native calls: on macOS they
	/// would read the grader's environment.
	fn confinements(
		&self,
		staged: &Staged,
		teacher: Vec<PathBuf>,
		student: Vec<PathBuf>,
	) -> Result<Option<[Confinement; 2]>, String> {
		// Asked even unsandboxed: an interpreter too old for the harness is refused here,
		// with its version, rather than by both sides failing to start.
		let interpreter = interpreter(&self.python_cmd)?;
		if !self.isolation.is_sandboxed() {
			return Ok(None);
		}
		let confine = |side: &Side, extra: Vec<PathBuf>| Confinement {
			read: covering(interpreter.read.iter().cloned().chain(extra).collect()),
			write: vec![side.work.clone(), side.home.clone(), side.tmp.clone()],
			exec: interpreter.exec.clone(),
			spawn: false,
			no_ffi: true,
		};
		let mut teacher = teacher;
		teacher.extend(
			staged
				.source
				.as_ref()
				.and_then(|s| s.parent())
				.map(Path::to_path_buf),
		);
		Ok(Some([
			confine(&staged.teacher, canonical(teacher)),
			confine(&staged.student, canonical(student)),
		]))
	}

	/// Run a staged unit — the teacher side, and the student side when `with_student` —
	/// end both at `deadline_secs`, and the student side past `memory`.
	#[allow(clippy::too_many_arguments)]
	async fn run_unit(
		&self,
		staged: &Staged,
		deadline_secs: u64,
		memory: Memory,
		bounds: Bounds,
		with_student: bool,
		confinements: Option<[Confinement; 2]>,
	) -> Finished {
		let [teacher_confinement, student_confinement] = match confinements {
			Some([teacher, student]) => [Some(teacher), Some(student)],
			None => [None, None],
		};
		let mut teacher_cmd = match self.command(
			&TEACHER_SCRIPT,
			&staged.teacher,
			deadline_secs,
			memory.total_mb(),
			teacher_confinement.as_ref(),
		) {
			Ok(cmd) => cmd,
			Err(e) => return Finished::spawn_failed(e),
		};
		teacher_cmd.arg(&staged.payload);
		let mut student = None;
		if with_student {
			// teacher → student, and student → teacher.
			let pipes = std::io::pipe().and_then(|a| Ok((a, std::io::pipe()?)));
			let ((to_student, from_teacher), (to_teacher, from_student)) = match pipes {
				Ok(pipes) => pipes,
				Err(e) => {
					return Finished::spawn_failed(format!("could not join the two sides: {e}"));
				}
			};
			let mut cmd = match self.command(
				&STUDENT_SCRIPT,
				&staged.student,
				deadline_secs,
				memory.total_mb(),
				student_confinement.as_ref(),
			) {
				Ok(cmd) => cmd,
				Err(e) => return Finished::spawn_failed(e),
			};
			cmd.stdin(std::process::Stdio::from(to_student))
				.stdout(std::process::Stdio::from(from_student))
				.stderr(std::process::Stdio::piped());
			// The command holds the parent's copies of the student's ends: they go with it.
			match cmd.spawn() {
				Ok(child) => student = Some(child),
				Err(e) => {
					return Finished::spawn_failed(format!(
						"could not start {}: {e}",
						self.python_cmd
					));
				}
			}
			drop(cmd);
			teacher_cmd
				.stdin(std::process::Stdio::from(to_teacher))
				.stdout(std::process::Stdio::from(from_teacher));
		} else {
			teacher_cmd
				.stdin(std::process::Stdio::null())
				.stdout(std::process::Stdio::null());
		}
		#[cfg(unix)]
		let records = {
			teacher_cmd.stderr(std::process::Stdio::piped());
			match record_pipe(&mut teacher_cmd) {
				Ok(pipe) => pipe,
				Err(e) => {
					return Finished::spawn_failed(format!(
						"could not open the record channel: {e}"
					));
				}
			}
		};
		#[cfg(not(unix))]
		teacher_cmd.stderr(std::process::Stdio::piped());

		let mut teacher = match teacher_cmd.spawn() {
			Ok(child) => child,
			Err(e) => {
				return Finished::spawn_failed(format!("could not start {}: {e}", self.python_cmd));
			}
		};
		// The parent's copies of every pipe end go, so each pipe ends when its side does.
		drop(teacher_cmd);
		#[cfg(unix)]
		let records = {
			let (reader, writer) = records;
			drop(writer);
			match tokio::net::unix::pipe::Receiver::from_owned_fd(reader.into()) {
				Ok(receiver) => receiver,
				Err(e) => {
					return Finished::spawn_failed(format!(
						"could not read the record channel: {e}"
					));
				}
			}
		};
		#[cfg(not(unix))]
		let records = teacher.stderr.take();

		// Killed however this future ends: finished, timed out, or dropped by a caller.
		let _teacher_group = GroupGuard::new(teacher.id());
		let student_group = GroupGuard::new(student.as_ref().and_then(Child::id));

		let reader = Arc::new(Mutex::new(Reader::new(bounds)));
		let teacher_tail = Arc::new(Mutex::new(Sink::tail(STDERR_TAIL)));
		let student_tail = Arc::new(Mutex::new(Sink::tail(STDERR_TAIL)));
		let mut pumps = tokio::task::JoinSet::new();
		#[cfg(unix)]
		pumps.spawn(pump_records(records, reader.clone()));
		#[cfg(not(unix))]
		if let Some(pipe) = records {
			pumps.spawn(pump_records(pipe, reader.clone()));
		}
		#[cfg(unix)]
		if let Some(pipe) = teacher.stderr.take() {
			pumps.spawn(pump(pipe, teacher_tail.clone()));
		}
		let over_memory = Arc::new(AtomicBool::new(false));
		let peak = Arc::new(AtomicU64::new(0));
		let mut watch = None;
		if let Some(child) = student.as_mut() {
			if let Some(pipe) = child.stderr.take() {
				pumps.spawn(pump(pipe, student_tail.clone()));
			}
			if let Some(pid) = child.id() {
				let (over, held) = (over_memory.clone(), peak.clone());
				let group = student_group.0;
				watch = Some(tokio::spawn(watch_memory(
					pid,
					group,
					memory.watched(),
					memory.every,
					over,
					held,
				)));
			}
		}

		let deadline = Duration::from_secs(deadline_secs);
		let teacher_exit = match tokio::time::timeout(deadline, teacher.wait()).await {
			Ok(Ok(status)) => exit_of(status),
			Ok(Err(e)) => Exit::Spawn(format!("lost the teacher side: {e}")),
			Err(_) => Exit::Deadline,
		};
		let exit = match student.as_mut() {
			None => Exit::Code(0),
			Some(child) => {
				let linger = if teacher_exit == Exit::Deadline {
					Duration::ZERO
				} else {
					LINGER
				};
				let ended = tokio::time::timeout(linger, child.wait()).await;
				let over = || over_memory.load(Ordering::SeqCst);
				match ended {
					Ok(Ok(_)) if over() => memory.exceeded(),
					Ok(Ok(status)) => exit_of(status),
					Ok(Err(e)) => Exit::Spawn(format!("lost the student side: {e}")),
					Err(_) if over() => memory.exceeded(),
					Err(_) if teacher_exit == Exit::Deadline => Exit::Deadline,
					Err(_) => Exit::Stopped,
				}
			}
		};
		// Its exit is settled: the watch has nothing left to end, and is not waited for.
		if let Some(watch) = watch {
			watch.abort();
		}
		// Whatever is left of either side — a hung student side, or a process either left
		// behind holding a pipe — goes now.
		drop(_teacher_group);
		drop(student_group);
		let _ = teacher.kill().await;
		if let Some(child) = student.as_mut() {
			let _ = child.kill().await;
		}

		// What was read stays read: a pipe held open past this only loses what comes after.
		let drain = tokio::time::sleep(Duration::from_secs(1));
		tokio::pin!(drain);
		loop {
			tokio::select! {
				joined = pumps.join_next() => if joined.is_none() { break },
				_ = &mut drain => {
					pumps.abort_all();
					break;
				}
			}
		}
		let records = std::mem::replace(
			&mut *reader.lock().expect("record reader poisoned"),
			Reader::new(bounds),
		)
		.finish();
		let tail = |sink: &Arc<Mutex<Sink>>| {
			String::from_utf8_lossy(&sink.lock().expect("stderr sink poisoned").bytes)
				.trim()
				.to_string()
		};
		let (teacher_err, student_err) = (tail(&teacher_tail), tail(&student_tail));
		let stderr = match (teacher_err.is_empty(), student_err.is_empty()) {
			(_, true) => teacher_err,
			(true, false) => format!("student side: {student_err}"),
			(false, false) => format!("{teacher_err}\nstudent side: {student_err}"),
		};
		Finished {
			records,
			exit,
			teacher_exit,
			stderr,
			peak: peak.load(Ordering::SeqCst),
		}
	}
}

/// A pipe whose write end the child sees as `RECORD_FD`. The write end stays open in the
/// parent until spawn, then must be dropped.
#[cfg(unix)]
fn record_pipe(cmd: &mut Command) -> std::io::Result<(std::io::PipeReader, std::io::PipeWriter)> {
	use std::os::fd::AsRawFd;
	let (reader, writer) = std::io::pipe()?;
	let fd = writer.as_raw_fd();
	// SAFETY: dup2 and fcntl are async-signal-safe, and this runs in the child between fork
	// and exec. std's pipe is close-on-exec; dup2 clears that on the copy, and when the pipe
	// already sits at RECORD_FD the flag is cleared directly.
	unsafe {
		cmd.pre_exec(move || {
			if fd == RECORD_FD {
				let flags = libc::fcntl(fd, libc::F_GETFD);
				if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
					return Err(std::io::Error::last_os_error());
				}
			} else if libc::dup2(fd, RECORD_FD) < 0 {
				return Err(std::io::Error::last_os_error());
			}
			Ok(())
		});
	}
	Ok((reader, writer))
}

/// Read frames as they arrive. Past a frame the stream may not hold, keep reading and
/// drop what comes, so a teacher side still writing can finish.
async fn pump_records<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, reader: Arc<Mutex<Reader>>) {
	use tokio::io::AsyncReadExt;
	let mut header = [0u8; 4];
	loop {
		if pipe.read_exact(&mut header).await.is_err() {
			break;
		}
		let size = u32::from_be_bytes(header) as usize;
		if !reader.lock().expect("record reader poisoned").admits(size) {
			let mut sink = tokio::io::sink();
			let _ = tokio::io::copy(&mut pipe, &mut sink).await;
			break;
		}
		let mut body = vec![0u8; size];
		// A frame cut off by a kill is the end of the stream, not a forgery.
		if pipe.read_exact(&mut body).await.is_err() {
			break;
		}
		reader.lock().expect("record reader poisoned").frame(&body);
	}
}

/// End the student side once it holds more than `limit` bytes: the operating system's
/// sandbox cannot bound its memory everywhere, so the grader watches it.
async fn watch_memory(
	pid: u32,
	group: Option<i32>,
	limit: u64,
	every: Duration,
	over: Arc<AtomicBool>,
	peak: Arc<AtomicU64>,
) {
	let mut tick = tokio::time::interval(every);
	while let Some(held) = resident_bytes(pid) {
		peak.fetch_max(held, Ordering::SeqCst);
		if held > limit {
			over.store(true, Ordering::SeqCst);
			if let Some(group) = group {
				kill_group(group);
			}
			return;
		}
		tick.tick().await;
	}
}

/// The memory `pid` holds, or None once it is gone.
#[cfg(target_os = "macos")]
fn resident_bytes(pid: u32) -> Option<u64> {
	let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
	let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
	// SAFETY: `info` is a properly sized, writable proc_taskinfo.
	let written = unsafe {
		libc::proc_pidinfo(
			i32::try_from(pid).ok()?,
			libc::PROC_PIDTASKINFO,
			0,
			(&mut info as *mut libc::proc_taskinfo).cast(),
			size,
		)
	};
	(written == size).then_some(info.pti_resident_size)
}

#[cfg(target_os = "linux")]
fn resident_bytes(pid: u32) -> Option<u64> {
	let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
	let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
	// SAFETY: sysconf only reads a constant.
	let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
	Some(pages.saturating_mul(u64::try_from(page).ok()?))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn resident_bytes(_pid: u32) -> Option<u64> {
	None
}

/// The limits for a unit: its CPU limit sits above its deadline, a backstop for one busy
/// core rather than a second, shorter timeout. Its address space (Linux) sits far above
/// the memory the grader's watch holds it to: a backstop for an allocation faster than the
/// watch, which a library reserving address space it never uses (OpenBLAS) must not hit.
/// Confined, it cannot start processes, so the per-user process limit — which counts
/// threads on Linux — only stops a thread bomb, and leaves a busy machine's own threads
/// room.
#[cfg(unix)]
fn sandbox_for(
	deadline_secs: u64,
	memory_mb: u64,
	confined: bool,
) -> crate::runner::sandbox::SandboxConfig {
	let defaults = crate::runner::sandbox::SandboxConfig::default();
	crate::runner::sandbox::SandboxConfig {
		cpu_secs: deadline_secs.saturating_add(1),
		mem_mb: memory_mb.saturating_mul(4).max(defaults.mem_mb),
		nproc: if confined {
			Some(crate::runner::sandbox::CONFINED_NPROC)
		} else {
			defaults.nproc
		},
		..defaults
	}
}

/// Every unit's process group still alive, so an interrupted grader can take them with it.
static LIVE_GROUPS: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Kill every unit process group still running. For a grader that is being interrupted:
/// the units run in their own groups, so the terminal's Ctrl-C does not reach them.
pub fn kill_all_units() {
	let groups = std::mem::take(&mut *LIVE_GROUPS.lock().expect("group registry poisoned"));
	for group in groups {
		kill_group(group);
	}
}

/// A side's process group: registered while it lives, killed when the guard drops.
struct GroupGuard(Option<i32>);

impl GroupGuard {
	fn new(pid: Option<u32>) -> Self {
		let group = pid.and_then(|p| i32::try_from(p).ok());
		if let Some(group) = group {
			LIVE_GROUPS
				.lock()
				.expect("group registry poisoned")
				.push(group);
		}
		Self(group)
	}
}

impl Drop for GroupGuard {
	fn drop(&mut self) {
		if let Some(group) = self.0 {
			kill_group(group);
			LIVE_GROUPS
				.lock()
				.expect("group registry poisoned")
				.retain(|g| *g != group);
		}
	}
}

#[cfg(unix)]
fn kill_group(group: i32) {
	// SAFETY: killpg only sends a signal; ESRCH (the group is already gone) is expected.
	unsafe {
		libc::killpg(group, libc::SIGKILL);
	}
}

#[cfg(not(unix))]
fn kill_group(_group: i32) {}

/// A pipe's contents, bounded: the last `cap` bytes.
struct Sink {
	bytes: Vec<u8>,
	cap: usize,
}

impl Sink {
	fn tail(cap: usize) -> Self {
		Self {
			bytes: Vec::new(),
			cap,
		}
	}

	fn push(&mut self, chunk: &[u8]) {
		self.bytes.extend_from_slice(chunk);
		if self.bytes.len() > self.cap {
			let excess = self.bytes.len() - self.cap;
			self.bytes.drain(..excess);
		}
	}
}

async fn pump<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, sink: Arc<Mutex<Sink>>) {
	use tokio::io::AsyncReadExt;
	let mut chunk = vec![0u8; 64 * 1024];
	while let Ok(n) = pipe.read(&mut chunk).await {
		if n == 0 {
			break;
		}
		sink.lock().expect("sink poisoned").push(&chunk[..n]);
	}
}

fn exit_of(status: std::process::ExitStatus) -> Exit {
	if let Some(code) = status.code() {
		return Exit::Code(code);
	}
	#[cfg(unix)]
	{
		use std::os::unix::process::ExitStatusExt;
		if let Some(signal) = status.signal() {
			return Exit::Signal(signal);
		}
	}
	Exit::Code(-1)
}

/// Remove a unit's directory off the async runtime.
async fn discard(staged: Staged) {
	let _ = tokio::task::spawn_blocking(move || drop(staged.root)).await;
}

/// The teacher side's payload, common to every unit: with each side's canary when sandboxed.
fn base_payload(mode: &str, staged: &Staged, sandboxed: bool) -> serde_json::Value {
	let canary = sandboxed.then_some(&staged.canary);
	serde_json::json!({
		"mode": mode,
		"records": RECORD_FD,
		"limits": LIMITS,
		"grace": GRACE_SECS,
		"startup": STARTUP_SECS,
		"canary": canary,
		"student_canary": canary,
	})
}

/// Each path that exists, resolved.
fn canonical(paths: Vec<PathBuf>) -> Vec<PathBuf> {
	paths
		.into_iter()
		.filter_map(|p| std::fs::canonicalize(p).ok())
		.collect()
}

/// What teacher code reads: the bundle, and the directories its modules import beside.
fn teacher_reads(bundle: &Path, imports: &[String]) -> Vec<PathBuf> {
	std::iter::once(bundle.to_path_buf())
		.chain(
			imports
				.iter()
				.filter_map(|i| Path::new(i).parent().map(Path::to_path_buf)),
		)
		.collect()
}

fn extend(mut base: serde_json::Value, more: serde_json::Value) -> serde_json::Value {
	if let (Some(base), serde_json::Value::Object(more)) = (base.as_object_mut(), more) {
		base.extend(more);
	}
	base
}

impl Executor for PythonExecutor {
	fn language(&self) -> &str {
		"python"
	}

	fn identity(&self) -> Result<String, String> {
		let bytes = std::fs::read(&self.python_cmd)
			.map_err(|e| format!("cannot fingerprint Python '{}': {e}", self.python_cmd))?;
		Ok(format!(
			"{}:{}",
			self.python_cmd,
			crate::runner::answers::digest(&bytes)
		))
	}

	async fn inspect(&self, spec: &TestSpec, timeout_secs: u64) -> Result<TeacherRuntime, String> {
		if spec.meta.imports.is_empty() && spec.meta.shared.is_empty() {
			return Ok(TeacherRuntime::default());
		}
		let data = spec
			.meta
			.data_files
			.iter()
			.map(|rel| (spec.dir.join(rel), rel.clone()))
			.collect::<Vec<_>>();
		let imports = spec.meta.imports.clone();
		let shared: Vec<(PathBuf, String)> = spec
			.meta
			.shared
			.iter()
			.map(|rel| (spec.dir.join(rel), rel.clone()))
			.collect();
		let teacher_shared: Vec<PathBuf> = shared.iter().map(|(src, _)| src.clone()).collect();
		let bundle = spec.dir.clone();
		let sandboxed = self.isolation.is_sandboxed();
		let staged = tokio::task::spawn_blocking(move || {
			stage(None, &data, &shared, |staged| {
				extend(
					base_payload("inspect", staged, sandboxed),
					serde_json::json!({
						"imports": imports,
						"shared": teacher_shared,
						"bundle": bundle,
					}),
				)
			})
		})
		.await
		.map_err(|e| e.to_string())?
		.map_err(|e| format!("could not stage the teacher modules: {e}"))?;
		let deadline = timeout_secs.saturating_add(STARTUP_SECS);
		let finished = match self.confinements(
			&staged,
			teacher_reads(&spec.dir, &spec.meta.imports),
			Vec::new(),
		) {
			Ok(confinements) => {
				self.run_unit(
					&staged,
					deadline,
					Memory {
						limit_mb: spec
							.meta
							.memory_mb
							.unwrap_or(crate::spec_loader::DEFAULT_MEMORY_MB),
						idle: 0,
						every: SAMPLE_EVERY,
					},
					INSPECT_BOUNDS,
					false,
					confinements,
				)
				.await
			}
			Err(e) => Finished::spawn_failed(e),
		};
		discard(staged).await;

		let records = finished.records;
		if let Some(fatal) = records.fatal {
			return Err(format!(
				"teacher module failed to import: {}: {}",
				fatal.error.type_name, fatal.error.message
			));
		}
		match records.inspect {
			Some(runtime) => Ok(runtime),
			None => Err(match finished.teacher_exit {
				Exit::Deadline => {
					format!("importing the teacher modules took longer than {timeout_secs}s")
				}
				Exit::Spawn(e) => e,
				other => format!(
					"the teacher modules could not be inspected ({other:?}): {}",
					finished.stderr.trim()
				),
			}),
		}
	}

	async fn run(&self, plan: &UnitPlan) -> UnitObservation {
		let idle = match self.idle_memory(plan).await {
			Ok(bytes) => bytes,
			Err(e) => {
				let mut obs = UnitObservation::not_started(Exit::Spawn(e));
				obs.sandboxed = self.isolation.is_sandboxed();
				return obs;
			}
		};
		self.observe(
			plan,
			Memory {
				limit_mb: plan.memory_mb,
				idle,
				every: SAMPLE_EVERY,
			},
		)
		.await
		.0
	}
}

impl PythonExecutor {
	/// What the student side of `plan` takes idle, in bytes: Python, the harness, the
	/// teacher's values and modules, and every module the submission imports — the most a
	/// unit whose student is ScriptMark's own was seen holding, sampled every 2 ms.
	/// `memory_mb` counts beyond it. Once per set of imports and what the teacher gives.
	#[cfg(any(target_os = "macos", target_os = "linux"))]
	async fn idle_memory(&self, plan: &UnitPlan) -> Result<u64, String> {
		let source = tokio::fs::read(&plan.file)
			.await
			.map_err(|e| format!("could not read {}: {e}", plan.file.display()))?;
		let sandboxed = self.isolation.is_sandboxed();
		let read = once(
			&SOURCE_IMPORTS,
			digest(&serde_json::json!({
				"python": self.python_cmd,
				"sandboxed": sandboxed,
				"source": String::from_utf8_lossy(&source),
			})),
		);
		let imports = read
			.get_or_try_init(|| async {
				let reader = Self {
					python_cmd: self.python_cmd.clone(),
					isolation: self.isolation,
				};
				tokio::task::spawn_blocking(move || reader.source_imports(&source))
					.await
					.map_err(|e| format!("reading the submission's imports panicked: {e}"))?
			})
			.await?
			.clone();
		let measured = once(
			&IDLE_MEMORY,
			digest(&serde_json::json!({
				"python": self.python_cmd,
				"sandboxed": sandboxed,
				"subject": plan.subject,
				"allowed_imports": plan.allowed_imports,
				"vars": &*plan.vars,
				"shared": plan.shared,
				"data_files": plan.data_files,
				"imports": imports,
			})),
		);
		measured
			.get_or_try_init(|| self.measure_idle_memory(plan, &imports))
			.await
			.copied()
	}

	/// Nothing to count it by where the student side's memory is not watched.
	#[cfg(not(any(target_os = "macos", target_os = "linux")))]
	async fn idle_memory(&self, _: &UnitPlan) -> Result<u64, String> {
		Ok(0)
	}

	/// The modules `source` imports, wherever in it: read by Python's own parser in a process
	/// of its own, confined and not watched — what parsing takes is the source's to decide,
	/// and must never be part of what a unit takes idle. Nothing in the source runs. One
	/// that does not parse, is past `IMPORTS_SOURCE_LIMIT` or takes too long imports nothing
	/// beforehand: its imports count.
	#[cfg(any(target_os = "macos", target_os = "linux"))]
	fn source_imports(&self, source: &[u8]) -> Result<Imports, String> {
		if source.len() > IMPORTS_SOURCE_LIMIT {
			return Ok(Vec::new());
		}
		let (_dir, scratch) = tempfile::Builder::new()
			.prefix("scriptmark-imports-")
			.tempdir()
			.and_then(|dir| std::fs::canonicalize(dir.path()).map(|path| (dir, path)))
			.map_err(|e| format!("could not stage the import reader: {e}"))?;
		let mut cmd = if self.isolation.is_sandboxed() {
			let confinement =
				teacher_confinement(&self.python_cmd, Vec::new(), vec![scratch.clone()], false)?;
			crate::runner::sandbox::confined(Path::new(&self.python_cmd), &confinement)?
		} else {
			std::process::Command::new(&self.python_cmd)
		};
		cmd.env_clear()
			.env("HOME", &scratch)
			.env("TMPDIR", &scratch)
			.current_dir(&scratch)
			.args(["-I", "-B", "-c", IMPORTS]);
		let ran = crate::checker::python_checker::run_with_timeout(
			cmd,
			Some(String::from_utf8_lossy(source).into_owned()),
			Duration::from_secs(STARTUP_SECS),
		)
		.map_err(|e| format!("could not start the import reader: {e}"))?;
		let Some(ran) = ran else {
			return Ok(Vec::new());
		};
		let found: Vec<(String, Vec<String>)> =
			serde_json::from_str(ran.stdout.trim()).map_err(|e| {
				format!(
					"the import reader printed no imports ({e}, {}): {}",
					ran.status,
					ran.stderr.trim()
				)
			})?;
		let mut imports: Imports = found
			.into_iter()
			.filter(|(name, names)| {
				crate::spec_loader::is_module_name(name)
					&& names.iter().all(|n| crate::spec_loader::is_module_name(n))
			})
			.collect();
		imports.sort();
		imports.dedup();
		Ok(imports)
	}

	#[cfg(any(target_os = "macos", target_os = "linux"))]
	async fn measure_idle_memory(
		&self,
		given: &UnitPlan,
		imports: &Imports,
	) -> Result<u64, String> {
		let dir = tempfile::tempdir().map_err(|e| format!("could not stage the idle unit: {e}"))?;
		let file = dir.path().join("idle.py");
		std::fs::write(&file, IDLE).map_err(|e| format!("could not stage the idle unit: {e}"))?;
		let plan = UnitPlan {
			subject: given.subject,
			functions: crate::matching::Functions::default(),
			file,
			script: None,
			imports: Vec::new(),
			vars: given.vars.clone(),
			data_files: given.data_files.clone(),
			bundle: given.bundle.clone(),
			shared: given.shared.clone(),
			memory_mb: *crate::spec_loader::MEMORY_MB.end(),
			allowed_imports: given.allowed_imports.clone(),
			load_timeout: given.load_timeout.max(STARTUP_SECS),
			setup: Vec::new(),
			steps: vec![crate::runner::executor::CallPlan {
				target: crate::models::Target::Function {
					name: "idle".into(),
				},
				args: vec![serde_json::json!(imports)],
				stdin: None,
				timeout: STARTUP_SECS,
				id: None,
				files: Vec::new(),
				check: None,
			}],
		};
		let memory = Memory {
			limit_mb: plan.memory_mb,
			idle: 0,
			every: Duration::from_millis(2),
		};
		let (obs, peak) = self.observe(&plan, memory).await;
		if let Exit::Spawn(e) = &obs.exit {
			return Err(e.clone());
		}
		let idled = obs.done
			&& obs.steps.first().is_some_and(|s| {
				matches!(s.outcome, crate::runner::executor::Outcome::Returned { .. })
			});
		if !idled || peak == 0 {
			return Err(format!(
				"could not measure what Python takes idle ({:?}): {}",
				obs.exit,
				obs.stderr.trim()
			));
		}
		Ok(peak)
	}

	/// Run `plan` with the student side held to `memory`: what it reported, and the most
	/// the student side was seen holding.
	async fn observe(&self, plan: &UnitPlan, memory: Memory) -> (UnitObservation, u64) {
		let data = plan.data_files.clone();
		let shared = plan.shared.clone();
		let file = plan.file.clone();
		let payload = {
			let plan = plan.clone();
			let sandboxed = self.isolation.is_sandboxed();
			move |staged: &Staged| {
				extend(
					base_payload("unit", staged, sandboxed),
					serde_json::json!({
						"role": match plan.subject {
							Subject::Student => "student",
							Subject::Reference => "reference",
						},
						"subject": staged
							.subject
							.as_deref()
							.and_then(Path::file_name)
							.map(|n| n.to_string_lossy().into_owned()),
						"source": staged.source,
						"script": plan.script.as_ref().map(|s| serde_json::json!({
							"timeout": s.timeout,
							"stdin": s.stdin,
							"files": s.files,
						})),
						"imports": plan.imports,
						"shared": plan.shared.iter().map(|(src, _)| src).collect::<Vec<_>>(),
						"student_shared": plan.shared.iter().map(|(_, rel)| rel).collect::<Vec<_>>(),
						"vars": &*plan.vars,
						"allowed_imports": plan.allowed_imports,
						"load_timeout": plan.load_timeout,
						"functions": plan.functions,
						"setup": plan.setup,
						"steps": plan.steps,
					}),
				)
			}
		};
		let staged =
			match tokio::task::spawn_blocking(move || stage(Some(&file), &data, &shared, payload))
				.await
			{
				Ok(Ok(staged)) => staged,
				Ok(Err(e)) => {
					let exit = Exit::Spawn(format!("could not stage the unit: {e}"));
					return (UnitObservation::not_started(exit), 0);
				}
				Err(e) => {
					let exit = Exit::Spawn(format!("staging panicked: {e}"));
					return (UnitObservation::not_started(exit), 0);
				}
			};
		debug_assert!(staged.subject.as_ref().is_some_and(|s| s.is_file()));

		// A reference implementation is teacher code: it may read the bundle, as teacher
		// code beside it may.
		let student_reads = if plan.subject == Subject::Reference {
			vec![plan.bundle.clone()]
		} else {
			Vec::new()
		};
		let finished = match self.confinements(
			&staged,
			teacher_reads(&plan.bundle, &plan.imports),
			student_reads,
		) {
			Ok(confinements) => {
				self.run_unit(
					&staged,
					plan.deadline_secs(),
					memory,
					record_bounds(plan),
					true,
					confinements,
				)
				.await
			}
			Err(e) => Finished::spawn_failed(e),
		};
		discard(staged).await;

		let records = finished.records;
		let obs = UnitObservation {
			ready: records.ready,
			load: records.load,
			setup: records.setup,
			steps: records.steps,
			checks: records.checks,
			fatal: records.fatal,
			stopped: records.stopped,
			done: records.done,
			exit: finished.exit,
			teacher_exit: finished.teacher_exit,
			protocol_error: records.protocol_error,
			stderr: finished.stderr,
			sandboxed: self.isolation.is_sandboxed(),
		};
		(obs, finished.peak)
	}
}

impl Default for PythonExecutor {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::runner::executor::CallPlan;

	/// The CPU limit is derived from the unit's own deadline — never the fixed 30 s it used
	/// to be, which a scenario of several long steps would outrun and be killed by.
	#[test]
	fn test_the_cpu_limit_sits_above_any_units_deadline() {
		for deadline in [3, 45, 4 * 10 + 2, 86_400 * 4] {
			assert!(sandbox_for(deadline, 256, true).cpu_secs > deadline);
		}
		assert_eq!(
			sandbox_for(u64::MAX, 256, true).cpu_secs,
			u64::MAX,
			"saturates"
		);
		assert_eq!(
			sandbox_for(3, 256, true).nproc,
			Some(crate::runner::sandbox::CONFINED_NPROC)
		);
		assert_eq!(sandbox_for(3, 256, false).nproc, Some(64));
	}

	/// Each record is bounded by the largest a call can make, and the stream by the number
	/// of records the plan can produce: a long scenario of large but legal values is never
	/// taken for a flood, and nothing is buffered on a guess.
	#[test]
	fn test_record_bounds_follow_the_plan() {
		let step = |files: usize| CallPlan {
			target: crate::models::Target::Function { name: "f".into() },
			args: Vec::new(),
			stdin: None,
			timeout: 1,
			id: None,
			files: vec!["out.txt".into(); files],
			check: None,
		};
		let plan = |steps: Vec<CallPlan>| UnitPlan {
			subject: Subject::Student,
			functions: crate::matching::Functions::default(),
			file: PathBuf::from("lab.py"),
			script: None,
			imports: Vec::new(),
			vars: Default::default(),
			data_files: Vec::new(),
			bundle: PathBuf::new(),
			shared: Vec::new(),
			memory_mb: crate::spec_loader::DEFAULT_MEMORY_MB,
			allowed_imports: Vec::new(),
			load_timeout: 1,
			setup: Vec::new(),
			steps,
		};
		let plain = record_bounds(&plan(vec![step(0); 20]));
		let filed = record_bounds(&plan(vec![step(2); 20]));
		// Measured: one call with a 4 MiB value, 64 KiB of escaped stdout, a failing check's
		// 64 KiB message and one 1 Mi-character file came to 10.75 MiB.
		assert!(filed.frame >= 43 << 18, "{}", filed.frame);
		assert!(filed.frame > plain.frame);
		assert_eq!(plain.count, filed.count);
		assert!(plain.count >= 2 * 20 + 3);
	}
}
