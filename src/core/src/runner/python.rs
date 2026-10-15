//! The Python backend. A unit runs as two processes joined by a pair of pipes: the teacher
//! side (`harness/teacher.py`) holds the teacher's modules and checkers, drives the plan
//! and writes the records; the student side (`harness/student.py`) holds only the
//! student's module. ScriptMark starts both, connects them to each other, and reads the
//! teacher side's records; it never relays what passes between them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Resolve a python command to an absolute path so env_clear() doesn't change
/// which interpreter runs. Falls back to the original string if resolution fails.
fn resolve_python_path(cmd: &str) -> String {
	if cmd.starts_with('/') {
		return cmd.to_string();
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
	memory_mb: u64,
}

impl PythonExecutor {
	pub fn new() -> Self {
		Self::with_python_cmd("python3")
	}

	pub fn with_python_cmd(python_cmd: impl Into<String>) -> Self {
		Self {
			python_cmd: resolve_python_path(&python_cmd.into()),
			memory_mb: crate::runner::sandbox::SandboxConfig::default().mem_mb,
		}
	}

	/// End the student side once it holds more than `mb` MiB.
	pub fn with_memory_limit(mut self, mb: u64) -> Self {
		self.memory_mb = mb;
		self
	}

	pub fn python_cmd(&self) -> &str {
		&self.python_cmd
	}
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
	payload: impl FnOnce(&Staged) -> serde_json::Value,
) -> std::io::Result<Staged> {
	let root = tempfile::Builder::new()
		.prefix("scriptmark-unit-")
		.tempdir()?;
	let teacher = side(root.path(), "teacher")?;
	let student = side(root.path(), "student")?;
	// Teacher code that opens a data file by its relative path finds it, as student code does.
	for (src, rel) in data_files {
		copy_into(src, &student.work.join(rel))?;
		copy_into(src, &teacher.work.join(rel))?;
	}
	let (copied, source) = match subject {
		Some(file) => {
			let name = file
				.file_name()
				.unwrap_or(std::ffi::OsStr::new("subject.py"));
			let dest = student.work.join(name);
			std::fs::copy(file, &dest)?;
			let sources = root.path().join("source");
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
}

impl Finished {
	fn spawn_failed(message: String) -> Self {
		Self {
			records: Records::default(),
			exit: Exit::Spawn(message.clone()),
			teacher_exit: Exit::Spawn(message),
			stderr: String::new(),
		}
	}
}

impl PythonExecutor {
	/// A command for one side: a cleared environment, the side's own `HOME` and `TMPDIR`,
	/// and the kernel's limits.
	fn command(&self, script: &str, side: &Side, deadline_secs: u64) -> Command {
		let mut cmd = Command::new(&self.python_cmd);
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
			apply_sandbox(&mut cmd, &sandbox_for(deadline_secs));
		}
		#[cfg(not(unix))]
		let _ = deadline_secs;
		cmd
	}

	/// Run a staged unit — the teacher side, and the student side when `with_student` —
	/// and end both at `deadline_secs`.
	async fn run_unit(
		&self,
		staged: &Staged,
		deadline_secs: u64,
		bounds: Bounds,
		with_student: bool,
	) -> Finished {
		let mut teacher_cmd = self.command(&TEACHER_SCRIPT, &staged.teacher, deadline_secs);
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
			let mut cmd = self.command(&STUDENT_SCRIPT, &staged.student, deadline_secs);
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
		if let Some(child) = student.as_mut() {
			if let Some(pipe) = child.stderr.take() {
				pumps.spawn(pump(pipe, student_tail.clone()));
			}
			if let Some(pid) = child.id() {
				let limit = self.memory_mb.saturating_mul(1 << 20);
				let over = over_memory.clone();
				let group = student_group.0;
				pumps.spawn(async move { watch_memory(pid, group, limit, over).await });
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
					Ok(Ok(_)) if over() => Exit::Memory,
					Ok(Ok(status)) => exit_of(status),
					Ok(Err(e)) => Exit::Spawn(format!("lost the student side: {e}")),
					Err(_) if over() => Exit::Memory,
					Err(_) if teacher_exit == Exit::Deadline => Exit::Deadline,
					Err(_) => Exit::Stopped,
				}
			}
		};
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
async fn watch_memory(pid: u32, group: Option<i32>, limit: u64, over: Arc<AtomicBool>) {
	let mut tick = tokio::time::interval(Duration::from_millis(50));
	while let Some(held) = resident_bytes(pid) {
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

/// The sandbox for a unit: its CPU limit sits above its deadline, a backstop for one busy
/// core rather than a second, shorter timeout.
fn sandbox_for(deadline_secs: u64) -> crate::runner::sandbox::SandboxConfig {
	crate::runner::sandbox::SandboxConfig {
		cpu_secs: deadline_secs.saturating_add(1),
		..Default::default()
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

/// The teacher side's payload, common to every unit of a spec.
fn base_payload(mode: &str) -> serde_json::Value {
	serde_json::json!({
		"mode": mode,
		"records": RECORD_FD,
		"limits": LIMITS,
		"grace": GRACE_SECS,
		"startup": STARTUP_SECS,
	})
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
		if spec.meta.imports.is_empty() {
			return Ok(TeacherRuntime::default());
		}
		let data = spec
			.meta
			.data_files
			.iter()
			.map(|rel| (spec.dir.join(rel), rel.clone()))
			.collect::<Vec<_>>();
		let payload = extend(
			base_payload("inspect"),
			serde_json::json!({ "imports": spec.meta.imports }),
		);
		let staged = tokio::task::spawn_blocking(move || stage(None, &data, |_| payload))
			.await
			.map_err(|e| e.to_string())?
			.map_err(|e| format!("could not stage the teacher modules: {e}"))?;
		let deadline = timeout_secs.saturating_add(STARTUP_SECS);
		let finished = self
			.run_unit(&staged, deadline, INSPECT_BOUNDS, false)
			.await;
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
		let data = plan.data_files.clone();
		let file = plan.file.clone();
		let payload = {
			let plan = plan.clone();
			move |staged: &Staged| {
				extend(
					base_payload("unit"),
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
		let staged = match tokio::task::spawn_blocking(move || stage(Some(&file), &data, payload))
			.await
		{
			Ok(Ok(staged)) => staged,
			Ok(Err(e)) => {
				return UnitObservation::not_started(Exit::Spawn(format!(
					"could not stage the unit: {e}"
				)));
			}
			Err(e) => {
				return UnitObservation::not_started(Exit::Spawn(format!("staging panicked: {e}")));
			}
		};
		debug_assert!(staged.subject.as_ref().is_some_and(|s| s.is_file()));

		let finished = self
			.run_unit(&staged, plan.deadline_secs(), record_bounds(plan), true)
			.await;
		discard(staged).await;

		let records = finished.records;
		UnitObservation {
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
		}
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
			assert!(sandbox_for(deadline).cpu_secs > deadline);
		}
		assert_eq!(sandbox_for(u64::MAX).cpu_secs, u64::MAX, "saturates");
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
