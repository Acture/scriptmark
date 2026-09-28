use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::models::{StudentFile, TestSpec};
use crate::runner::executor::{Executor, Exit, Subject, TeacherRuntime, UnitObservation, UnitPlan};
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
		use tokio::io::{AsyncReadExt, AsyncWriteExt};

		let sandbox = crate::runner::sandbox::SandboxConfig {
			// Above the deadline, so the harness's own timers always fire first.
			cpu_secs: deadline_secs + 1,
			..Default::default()
		};
		let mut cmd = Command::new(&self.python_cmd);
		cmd.env_clear()
			.env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
			.env("HOME", "/tmp")
			.env("PYTHONDONTWRITEBYTECODE", "1")
			.env("PYTHONIOENCODING", "utf-8")
			.arg("-c")
			.arg(HARNESS)
			.arg(&staged.payload)
			.current_dir(&staged.work)
			.stdin(if stdin.is_some() {
				std::process::Stdio::piped()
			} else {
				std::process::Stdio::null()
			})
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.kill_on_drop(true);
		#[cfg(unix)]
		apply_sandbox(&mut cmd, &sandbox);

		let mut child = match cmd.spawn() {
			Ok(child) => child,
			Err(e) => {
				return Finished {
					stdout: String::new(),
					stderr: String::new(),
					exit: Exit::Spawn(format!("could not start {}: {e}", self.python_cmd)),
				};
			}
		};

		if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
			tokio::spawn(async move {
				// A script that never reads its stdin closes the pipe; that is not an error.
				let _ = pipe.write_all(text.as_bytes()).await;
			});
		}
		let mut out = child.stdout.take();
		let mut err = child.stderr.take();
		let stdout_task = tokio::spawn(async move {
			let mut buf = Vec::new();
			if let Some(pipe) = out.as_mut() {
				let _ = pipe.read_to_end(&mut buf).await;
			}
			buf
		});
		let stderr_task = tokio::spawn(async move {
			let mut buf = Vec::new();
			if let Some(pipe) = err.as_mut() {
				let _ = pipe.read_to_end(&mut buf).await;
			}
			buf
		});

		let exit =
			match tokio::time::timeout(std::time::Duration::from_secs(deadline_secs), child.wait())
				.await
			{
				Ok(Ok(status)) => exit_of(status),
				Ok(Err(e)) => Exit::Spawn(format!("lost the process: {e}")),
				Err(_) => {
					let _ = child.kill().await;
					Exit::Deadline
				}
			};

		// The pipes close with the process; a grandchild holding one gets a moment, no more.
		let drain = std::time::Duration::from_secs(1);
		let stdout = tokio::time::timeout(drain, stdout_task)
			.await
			.ok()
			.and_then(Result::ok)
			.unwrap_or_default();
		let stderr = tokio::time::timeout(drain, stderr_task)
			.await
			.ok()
			.and_then(Result::ok)
			.unwrap_or_default();
		let stderr = String::from_utf8_lossy(&stderr);
		let tail_from = stderr
			.char_indices()
			.rev()
			.nth(STDERR_TAIL)
			.map_or(0, |(i, _)| i);
		Finished {
			stdout: String::from_utf8_lossy(&stdout).into_owned(),
			stderr: stderr[tail_from..].to_string(),
			exit,
		}
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
		// Today's chain rule: [meta] function, else the first function any case names.
		let hint = spec.meta.function.as_deref().or_else(|| {
			spec.cases
				.iter()
				.chain(spec.scenarios.iter().flat_map(|s| s.steps.iter()))
				.find_map(|c| c.function.as_deref())
		});
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
			Some(exports) => Ok(TeacherRuntime { exports }),
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

		let records = crate::runner::records::parse(&finished.stdout, &nonce);
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
