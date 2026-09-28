use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::process::Command;

use crate::models::{StudentFile, TestSpec};
use crate::runner::executor::{
	Executor, Exit, ProtocolError, Subject, TeacherRuntime, UnitObservation, UnitPlan,
};
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
}

impl PythonExecutor {
	pub fn new() -> Self {
		Self::with_python_cmd("python3")
	}

	pub fn with_python_cmd(python_cmd: impl Into<String>) -> Self {
		Self {
			python_cmd: resolve_python_path(&python_cmd.into()),
		}
	}

	pub fn python_cmd(&self) -> &str {
		&self.python_cmd
	}

	/// Find the student file matching the spec's file pattern.
	///
	/// Strategy (scored, best wins):
	/// 1. Exact suffix match (100) → highest confidence
	/// 2. Strip numeric prefixes, compare stems (80-100)
	/// 3. Stem-contains (40-60)
	/// 4. Function-definition scan (+200 bonus) → if spec has function hint,
	///    files containing `def function_name` get a large boost
	fn find_student_file_with_hint<'a>(
		&self,
		student_files: &'a [StudentFile],
		pattern: &str,
		function_hint: Option<&str>,
	) -> Option<&'a StudentFile> {
		let pattern_stem = Path::new(pattern)
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or(pattern);

		// 1. Exact suffix match — highest confidence
		if let Some(f) = student_files.iter().find(|f| {
			f.path
				.file_name()
				.and_then(|n| n.to_str())
				.is_some_and(|n| n.ends_with(pattern))
		}) {
			return Some(f);
		}

		// Helper: strip numeric prefix segments (SID_uploadID_fileID_)
		// "21300110043_171469_6012331_Lab3_2-2.py" → "Lab3_2-2.py"
		fn extract_actual_name(filename: &str) -> &str {
			let mut rest = filename;
			loop {
				if let Some(idx) = rest.find('_') {
					let prefix = &rest[..idx];
					if prefix.chars().all(|c| c.is_ascii_digit()) {
						rest = &rest[idx + 1..];
						continue;
					}
				}
				break;
			}
			rest
		}

		// 2. Score all candidates by filename similarity + function content
		let mut scored: Vec<(&'a StudentFile, u32)> = student_files
			.iter()
			.filter_map(|f| {
				let filename = f.path.file_name()?.to_str()?;
				let actual = extract_actual_name(filename);
				let actual_stem = Path::new(actual)
					.file_stem()
					.and_then(|s| s.to_str())
					.unwrap_or(actual);

				let mut score: u32 = 0;

				// --- Filename similarity ---
				if actual_stem == pattern_stem {
					score += 100;
				} else if actual_stem.starts_with(pattern_stem) {
					score += 80;
				} else if actual.contains(pattern_stem) {
					score += 60;
				} else if filename.contains(pattern_stem) {
					score += 40;
				}

				// --- Function definition scan (highest priority tiebreaker) ---
				if let Some(func_name) = function_hint
					&& let Ok(content) = std::fs::read_to_string(&f.path)
				{
					let needle = format!("def {func_name}");
					if content.contains(&needle) {
						score += 200; // trumps filename-only matches
					}
				}

				if score > 0 { Some((f, score)) } else { None }
			})
			.collect();

		scored.sort_by_key(|a| std::cmp::Reverse(a.1));

		scored.first().map(|(f, _)| *f)
	}
}

/// The one harness every unit runs under. See `harness.py`.
const HARNESS: &str = include_str!("harness.py");

/// The tail of stderr kept for diagnosing a crash.
const STDERR_TAIL: usize = 4096;

/// The descriptor the harness writes records to. On unix it is a pipe of its own, so
/// nothing a student does to stdout — rewrapping it, writing to `sys.__stdout__`, printing
/// from a thread — can reach the records. Elsewhere it is stdout, framed by the nonce.
#[cfg(unix)]
const RECORD_FD: i32 = 3;
#[cfg(not(unix))]
const RECORD_FD: i32 = 1;

/// More than this on the record channel is not the harness writing: its records carry at
/// most a 4 MiB value, 64 KiB of stdout and 1 MiB per observed file per call.
const STDOUT_CAP: usize = 64 * 1024 * 1024;

/// A unit's private directory: the payload beside a working directory the student runs in.
struct Staged {
	/// Removed on drop.
	root: tempfile::TempDir,
	work: PathBuf,
	payload: PathBuf,
	/// The subject file's copy inside `work`.
	subject: PathBuf,
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

fn stage(
	subject: Option<&Path>,
	data_files: &[(PathBuf, String)],
	payload: impl FnOnce(Option<&Path>) -> serde_json::Value,
) -> std::io::Result<Staged> {
	let root = tempfile::Builder::new()
		.prefix("scriptmark-unit-")
		.tempdir()?;
	let work = root.path().join("work");
	std::fs::create_dir(&work)?;
	for (src, rel) in data_files {
		copy_into(src, &work.join(rel))?;
	}
	let copied = match subject {
		Some(file) => {
			let name = file
				.file_name()
				.unwrap_or(std::ffi::OsStr::new("subject.py"));
			let dest = work.join(name);
			std::fs::copy(file, &dest)?;
			Some(dest)
		}
		None => None,
	};
	let payload_path = root.path().join("payload.json");
	std::fs::write(&payload_path, payload(copied.as_deref()).to_string())?;
	Ok(Staged {
		work,
		payload: payload_path,
		subject: copied.unwrap_or_default(),
		root,
	})
}

fn nonce() -> String {
	use rand::Rng;
	let bytes: [u8; 16] = rand::rng().random();
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a finished harness process left behind.
struct Finished {
	stdout: String,
	/// The process wrote more than `STDOUT_CAP` to the record channel.
	stdout_overflow: bool,
	stderr: String,
	exit: Exit,
}

impl PythonExecutor {
	/// Run the harness on a staged unit, killing it at `deadline_secs`.
	async fn run_harness(
		&self,
		staged: &Staged,
		deadline_secs: u64,
		stdin: Option<String>,
	) -> Finished {
		use tokio::io::AsyncWriteExt;

		let mut cmd = Command::new(&self.python_cmd);
		cmd.env_clear()
			.env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
			.env("HOME", "/tmp")
			.env("PYTHONDONTWRITEBYTECODE", "1")
			.env("PYTHONIOENCODING", "utf-8")
			// Keep the unit's directory, which holds the student's file, off sys.path.
			.env("PYTHONSAFEPATH", "1")
			.arg("-c")
			.arg(HARNESS)
			.arg(&staged.payload)
			.current_dir(&staged.work)
			.stdin(if stdin.is_some() {
				std::process::Stdio::piped()
			} else {
				std::process::Stdio::null()
			})
			.stderr(std::process::Stdio::piped())
			.kill_on_drop(true);
		let spawn_failed = |message: String| Finished {
			stdout: String::new(),
			stdout_overflow: false,
			stderr: String::new(),
			exit: Exit::Spawn(message),
		};
		#[cfg(unix)]
		let records = {
			// Its own process group, so anything it spawns dies with it.
			cmd.process_group(0);
			apply_sandbox(&mut cmd, &sandbox_for(deadline_secs));
			// The student's stdout goes nowhere: what they print during a call is captured
			// in-process, and the records have a pipe of their own.
			cmd.stdout(std::process::Stdio::null());
			match record_pipe(&mut cmd) {
				Ok(pipe) => pipe,
				Err(e) => return spawn_failed(format!("could not open the record channel: {e}")),
			}
		};
		#[cfg(not(unix))]
		cmd.stdout(std::process::Stdio::piped());

		let mut child = match cmd.spawn() {
			Ok(child) => child,
			Err(e) => return spawn_failed(format!("could not start {}: {e}", self.python_cmd)),
		};
		// The parent's copy of the write end goes, so the channel ends when the unit does.
		#[cfg(unix)]
		let records = {
			let (reader, writer) = records;
			drop(writer);
			match tokio::net::unix::pipe::Receiver::from_owned_fd(reader.into()) {
				Ok(receiver) => receiver,
				Err(e) => return spawn_failed(format!("could not read the record channel: {e}")),
			}
		};
		#[cfg(not(unix))]
		let records = child.stdout.take();
		// Killed however this future ends: finished, timed out, or dropped by a caller.
		let _group = GroupGuard::new(child.id());

		if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
			tokio::spawn(async move {
				// A script that never reads its stdin closes the pipe; that is not an error.
				let _ = pipe.write_all(text.as_bytes()).await;
			});
		}
		let stdout = Arc::new(Mutex::new(Sink::head(STDOUT_CAP)));
		let stderr = Arc::new(Mutex::new(Sink::tail(STDERR_TAIL)));
		let mut pumps = tokio::task::JoinSet::new();
		#[cfg(unix)]
		pumps.spawn(pump(records, stdout.clone()));
		#[cfg(not(unix))]
		if let Some(pipe) = records {
			pumps.spawn(pump(pipe, stdout.clone()));
		}
		if let Some(pipe) = child.stderr.take() {
			pumps.spawn(pump(pipe, stderr.clone()));
		}

		let exit =
			match tokio::time::timeout(std::time::Duration::from_secs(deadline_secs), child.wait())
				.await
			{
				Ok(Ok(status)) => exit_of(status),
				Ok(Err(e)) => Exit::Spawn(format!("lost the process: {e}")),
				Err(_) => Exit::Deadline,
			};
		// Whatever is left of the group — the harness at its deadline, or a process a student
		// left behind holding the pipes — goes now.
		_group.kill();
		let _ = child.kill().await;

		// What was read stays read: a pipe held open past this only loses what comes after.
		let drain = tokio::time::sleep(std::time::Duration::from_secs(1));
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
		let (stdout, stdout_overflow) = {
			let sink = stdout.lock().expect("stdout sink poisoned");
			(
				String::from_utf8_lossy(&sink.bytes).into_owned(),
				sink.overflow,
			)
		};
		let stderr = String::from_utf8_lossy(&stderr.lock().expect("stderr sink poisoned").bytes)
			.into_owned();
		Finished {
			stdout,
			stdout_overflow,
			stderr,
			exit,
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

/// A unit's process group: registered while it lives, killed when the guard drops.
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

	fn kill(&self) {
		if let Some(group) = self.0 {
			kill_group(group);
		}
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

/// A pipe's contents, bounded: the first `cap` bytes, or only the last.
struct Sink {
	bytes: Vec<u8>,
	cap: usize,
	keep_tail: bool,
	overflow: bool,
}

impl Sink {
	fn head(cap: usize) -> Self {
		Self {
			bytes: Vec::new(),
			cap,
			keep_tail: false,
			overflow: false,
		}
	}

	fn tail(cap: usize) -> Self {
		Self {
			keep_tail: true,
			..Self::head(cap)
		}
	}

	fn push(&mut self, chunk: &[u8]) {
		if self.keep_tail {
			self.bytes.extend_from_slice(chunk);
			if self.bytes.len() > self.cap {
				let excess = self.bytes.len() - self.cap;
				self.bytes.drain(..excess);
			}
		} else {
			let room = self.cap.saturating_sub(self.bytes.len());
			self.bytes
				.extend_from_slice(&chunk[..room.min(chunk.len())]);
			self.overflow |= chunk.len() > room;
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

impl Executor for PythonExecutor {
	fn language(&self) -> &str {
		"python"
	}

	fn locate<'a>(&self, files: &'a [StudentFile], spec: &TestSpec) -> Option<&'a StudentFile> {
		// The hint chain mode used: the first function a case names, else [meta] function.
		// (Per-case specs named no case function, so this is their hint too.)
		let hint = spec
			.cases
			.iter()
			.chain(spec.scenarios.iter().flat_map(|s| s.steps.iter()))
			.find_map(|c| c.function.as_deref())
			.or(spec.meta.function.as_deref());
		self.find_student_file_with_hint(files, &spec.meta.file, hint)
	}

	async fn inspect(&self, spec: &TestSpec, timeout_secs: u64) -> Result<TeacherRuntime, String> {
		if spec.meta.imports.is_empty() {
			return Ok(TeacherRuntime::default());
		}
		let nonce = nonce();
		let data = spec
			.meta
			.data_files
			.iter()
			.map(|rel| (spec.dir.join(rel), rel.clone()))
			.collect::<Vec<_>>();
		let payload = serde_json::json!({
			"nonce": nonce,
			"channel": RECORD_FD,
			"mode": "inspect",
			"imports": spec.meta.imports,
		});
		let staged = tokio::task::spawn_blocking(move || stage(None, &data, |_| payload))
			.await
			.map_err(|e| e.to_string())?
			.map_err(|e| format!("could not stage the teacher modules: {e}"))?;
		let finished = self.run_harness(&staged, timeout_secs + 2, None).await;
		discard(staged).await;

		let records = crate::runner::records::parse(&finished.stdout, &nonce);
		if let Some(fatal) = records.fatal {
			return Err(format!(
				"teacher module failed to import: {}: {}",
				fatal.error.type_name, fatal.error.message
			));
		}
		match records.inspect {
			Some(runtime) => Ok(runtime),
			None => Err(match finished.exit {
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
		let nonce = nonce();
		let data = plan.data_files.clone();
		let file = plan.file.clone();
		let payload = {
			let nonce = nonce.clone();
			let plan = plan.clone();
			move |subject: Option<&Path>| {
				serde_json::json!({
					"nonce": nonce,
					"channel": RECORD_FD,
					"mode": "unit",
					"student": subject,
					"script": plan.script.as_ref().map(|s| serde_json::json!({
						"timeout": s.timeout,
						"files": s.files,
					})),
					"imports": plan.imports,
					"vars": &*plan.vars,
					"allowed_imports": plan.allowed_imports,
					"load_timeout": plan.load_timeout,
					"subject": plan.subject,
					"lookup": match plan.subject {
						Subject::Student => "fuzzy",
						Subject::Reference => "exact",
					},
					"setup": plan.setup,
					"steps": plan.steps,
				})
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
		debug_assert!(staged.subject.is_file());

		let stdin = plan
			.script
			.as_ref()
			.map(|s| s.stdin.clone().unwrap_or_default());
		let finished = self.run_harness(&staged, plan.deadline_secs(), stdin).await;
		discard(staged).await;

		let mut records = crate::runner::records::parse(&finished.stdout, &nonce);
		if finished.stdout_overflow && records.protocol_error.is_none() {
			records.protocol_error = Some(ProtocolError::Flooded(format!(
				"more than {} MiB was written to the record channel",
				STDOUT_CAP >> 20
			)));
		}
		UnitObservation {
			ready: records.ready,
			load: records.load,
			setup: records.setup,
			steps: records.steps,
			checks: records.checks,
			fatal: records.fatal,
			done: records.done,
			exit: finished.exit,
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

	/// The CPU limit is derived from the unit's own deadline — never the fixed 30 s it used
	/// to be, which a scenario of several long steps would outrun and be killed by.
	#[test]
	fn test_the_cpu_limit_sits_above_any_units_deadline() {
		for deadline in [3, 45, 4 * 10 + 2, 86_400 * 4] {
			assert!(sandbox_for(deadline).cpu_secs > deadline);
		}
		assert_eq!(sandbox_for(u64::MAX).cpu_secs, u64::MAX, "saturates");
	}
}
