use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::{CheckError, CheckInput, CheckOutput, Checker};

/// Checker that runs a Python verification script.
///
/// Protocol:
/// - stdin:  JSON `{"result": ..., "expected": ..., "context": {...}}`
/// - stdout: JSON `{"pass": true/false, "message": "..."}`
///
/// The script's existence is checked when the spec loads, so a script that then times out,
/// crashes or prints no verdict did so on this student's value: the answer is rejected.
///
/// It runs with a cleared environment and a private directory for its working directory,
/// `HOME` and `TMPDIR`; confined, in the teacher's sandbox, it reads only its interpreter
/// and the places it is given — the test bundle — and writes only that directory.
pub struct PythonChecker {
	pub script_path: PathBuf,
	pub python_cmd: String,
	pub timeout_secs: u64,
	/// What it may read besides its interpreter, when it runs in the teacher's sandbox.
	pub confined: Option<Vec<PathBuf>>,
}

impl PythonChecker {
	pub fn new(script_path: impl Into<PathBuf>) -> Self {
		Self {
			script_path: script_path.into(),
			python_cmd: "python3".to_string(),
			timeout_secs: 10,
			confined: None,
		}
	}

	pub fn with_python_cmd(mut self, cmd: impl Into<String>) -> Self {
		self.python_cmd = cmd.into();
		self
	}

	/// Run in the teacher's sandbox, reading `read` besides the interpreter.
	pub fn confined(mut self, read: Vec<PathBuf>) -> Self {
		self.confined = Some(read);
		self
	}

	fn command(&self, scratch: &Path) -> Result<Command, String> {
		let mut cmd = match &self.confined {
			Some(read) => {
				let confinement = crate::runner::python::teacher_confinement(
					&self.python_cmd,
					read.clone(),
					vec![scratch.to_path_buf()],
					false,
				)?;
				crate::runner::sandbox::confined(Path::new(&self.python_cmd), &confinement)?
			}
			None => Command::new(&self.python_cmd),
		};
		cmd.env_clear()
			.env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
			.env("HOME", scratch)
			.env("TMPDIR", scratch)
			.current_dir(scratch)
			// No user site-packages and no bytecode beside the script; the script's own
			// directory stays on sys.path, for the helpers beside it.
			.args(["-s", "-B"])
			.arg(&self.script_path);
		#[cfg(windows)]
		if let Some(root) = std::env::var_os("SYSTEMROOT") {
			cmd.env("SYSTEMROOT", root);
		}
		Ok(cmd)
	}
}

impl Checker for PythonChecker {
	fn check(&self, input: &CheckInput) -> Result<CheckOutput, CheckError> {
		let script = self.script_path.display();
		let input_json = serde_json::to_string(input)
			.map_err(|e| CheckError::environment(format!("could not encode checker input: {e}")))?;

		let scratch = tempfile::Builder::new()
			.prefix("scriptmark-check-")
			.tempdir()
			.and_then(|dir| std::fs::canonicalize(dir.path()).map(|path| (dir, path)))
			.map_err(|e| {
				CheckError::environment(format!("could not stage checker '{script}': {e}"))
			})?;
		let cmd = self.command(&scratch.1).map_err(|e| {
			CheckError::environment(format!("cannot sandbox checker '{script}': {e}"))
		})?;
		let ran = run_with_timeout(
			cmd,
			Some(input_json),
			Duration::from_secs(self.timeout_secs),
		)
		.map_err(|e| {
			CheckError::environment(format!(
				"could not start checker '{script}': {}",
				crate::runner::sandbox::spawn_error(&e)
			))
		})?;
		let Some(Ran {
			status,
			stdout,
			stderr,
		}) = ran
		else {
			return Err(CheckError::student(format!(
				"checker '{script}' timed out after {}s",
				self.timeout_secs
			)));
		};
		if !status.success() && stdout.trim().is_empty() {
			return Err(CheckError::student(format!(
				"checker '{script}' exited with {status}: {}",
				stderr.trim()
			)));
		}
		serde_json::from_str::<CheckOutput>(stdout.trim()).map_err(|e| {
			CheckError::student(format!(
				"checker '{script}' printed no verdict ({e}): {}",
				stdout.trim()
			))
		})
	}
}

/// What a finished tool left: how it ended, and what it printed.
pub(crate) struct Ran {
	pub status: std::process::ExitStatus,
	pub stdout: String,
	pub stderr: String,
}

/// Run `cmd` with `stdin` written to it, killing it at `timeout`: `None` then. Its input is
/// written and its output read on their own threads, so a tool that ignores its input, or
/// prints more than a pipe holds, cannot wedge the grader until the timeout.
pub(crate) fn run_with_timeout(
	mut cmd: Command,
	stdin: Option<String>,
	timeout: Duration,
) -> std::io::Result<Option<Ran>> {
	#[cfg(unix)]
	crate::runner::sandbox::limit_tool(&mut cmd, timeout.as_secs());
	let mut child = cmd
		.stdin(if stdin.is_some() {
			Stdio::piped()
		} else {
			Stdio::null()
		})
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()?;
	if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
		std::thread::spawn(move || {
			// A tool that exits without reading its input closes the pipe; its output decides.
			let _ = pipe.write_all(text.as_bytes());
		});
	}
	let drain = |pipe: Option<Box<dyn Read + Send>>| {
		std::thread::spawn(move || {
			let mut buf = String::new();
			if let Some(mut pipe) = pipe {
				let _ = pipe.read_to_string(&mut buf);
			}
			buf
		})
	};
	let stdout = drain(
		child
			.stdout
			.take()
			.map(|p| Box::new(p) as Box<dyn Read + Send>),
	);
	let stderr = drain(
		child
			.stderr
			.take()
			.map(|p| Box::new(p) as Box<dyn Read + Send>),
	);
	// Its own process group: whatever it started goes with it, so nothing left behind
	// holds its pipes open.
	let group = child.id();
	let end_group = || {
		#[cfg(unix)]
		if let Ok(group) = i32::try_from(group) {
			// SAFETY: killpg only sends a signal; the group may already be gone.
			unsafe {
				libc::killpg(group, libc::SIGKILL);
			}
		}
		#[cfg(not(unix))]
		let _ = group;
	};
	let Some(status) = child.wait_timeout(timeout)? else {
		end_group();
		let _ = child.kill();
		let _ = child.wait();
		return Ok(None);
	};
	end_group();
	Ok(Some(Ran {
		status,
		stdout: stdout.join().unwrap_or_default(),
		stderr: stderr.join().unwrap_or_default(),
	}))
}

// wait_timeout is not in std — implement using a thread
trait WaitTimeout {
	fn wait_timeout(
		&mut self,
		timeout: Duration,
	) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for std::process::Child {
	fn wait_timeout(
		&mut self,
		timeout: Duration,
	) -> std::io::Result<Option<std::process::ExitStatus>> {
		use std::thread;

		let start = std::time::Instant::now();
		let poll_interval = Duration::from_millis(10);

		loop {
			match self.try_wait()? {
				Some(status) => {
					return Ok(Some(status));
				}
				None => {
					if start.elapsed() >= timeout {
						return Ok(None);
					}
					thread::sleep(poll_interval.min(timeout - start.elapsed()));
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn write_checker_script(dir: &std::path::Path, name: &str, code: &str) -> PathBuf {
		let path = dir.join(name);
		std::fs::write(&path, code).unwrap();
		path
	}

	#[test]
	fn test_python_checker_pass() {
		let dir = tempfile::tempdir().unwrap();
		let script = write_checker_script(
			dir.path(),
			"check_pass.py",
			r#"
import sys, json
data = json.load(sys.stdin)
result = data["result"]
print(json.dumps({"pass": result > 0, "message": "" if result > 0 else "not positive"}))
"#,
		);

		let checker = PythonChecker::new(&script);
		let output = checker
			.check(&CheckInput {
				result: json!(42),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap();
		assert!(output.pass);
	}

	#[test]
	fn test_python_checker_fail() {
		let dir = tempfile::tempdir().unwrap();
		let script = write_checker_script(
			dir.path(),
			"check_fail.py",
			r#"
import sys, json
data = json.load(sys.stdin)
print(json.dumps({"pass": False, "message": "custom failure message"}))
"#,
		);

		let checker = PythonChecker::new(&script);
		let output = checker
			.check(&CheckInput {
				result: json!(0),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap();
		assert!(!output.pass);
		assert_eq!(output.message, "custom failure message");
	}

	#[test]
	fn test_python_checker_script_error() {
		let dir = tempfile::tempdir().unwrap();
		let script =
			write_checker_script(dir.path(), "check_error.py", "raise Exception('boom')\n");

		let checker = PythonChecker::new(&script);
		let output = checker
			.check(&CheckInput {
				result: json!(1),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap_err();
		assert_eq!(output.fault, crate::models::Fault::Student);
		assert!(output.message.contains("exited with"));
	}

	#[test]
	fn test_a_python_checker_that_hangs_or_gives_no_verdict_rejects_the_answer() {
		let dir = tempfile::tempdir().unwrap();
		let input = CheckInput {
			result: json!(1),
			expected: json!(null),
			context: json!({}),
		};
		let hangs = PythonChecker {
			timeout_secs: 1,
			..PythonChecker::new(write_checker_script(
				dir.path(),
				"hang.py",
				"import time\ntime.sleep(30)\n",
			))
		};
		let output = hangs.check(&input).unwrap_err();
		assert_eq!(output.fault, crate::models::Fault::Student);
		assert!(output.message.contains("timed out"));

		let mute = PythonChecker::new(write_checker_script(dir.path(), "mute.py", "print('hi')\n"));
		let output = mute.check(&input).unwrap_err();
		assert_eq!(output.fault, crate::models::Fault::Student);
		assert!(output.message.contains("no verdict"));
	}

	#[test]
	fn test_python_checker_missing_script() {
		let checker = PythonChecker::new("/nonexistent/checker.py");
		let output = checker
			.check(&CheckInput {
				result: json!(1),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap_err();
		assert_eq!(
			output.fault,
			crate::models::Fault::Student,
			"python ran; spec loading is what refuses a missing script"
		);
		assert!(output.message.contains("exited with"));
	}
}
