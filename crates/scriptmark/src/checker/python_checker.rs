use std::io::{Read, Write};
use std::path::PathBuf;
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
pub struct PythonChecker {
	pub script_path: PathBuf,
	pub python_cmd: String,
	pub timeout_secs: u64,
}

impl PythonChecker {
	pub fn new(script_path: impl Into<PathBuf>) -> Self {
		Self {
			script_path: script_path.into(),
			python_cmd: "python3".to_string(),
			timeout_secs: 10,
		}
	}

	pub fn with_python_cmd(mut self, cmd: impl Into<String>) -> Self {
		self.python_cmd = cmd.into();
		self
	}
}

impl Checker for PythonChecker {
	fn check(&self, input: &CheckInput) -> Result<CheckOutput, CheckError> {
		let script = self.script_path.display();
		let input_json = serde_json::to_string(input)
			.map_err(|e| CheckError::environment(format!("could not encode checker input: {e}")))?;

		let mut child = Command::new(&self.python_cmd)
			.arg(&self.script_path)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.map_err(|e| {
				CheckError::environment(format!("could not start checker '{script}': {e}"))
			})?;

		// Write and read on their own threads: a checker that ignores its input, or prints
		// more than a pipe holds, must not wedge the grader until the timeout.
		if let Some(mut stdin) = child.stdin.take() {
			std::thread::spawn(move || {
				// A checker that exits without reading its input closes the pipe; its output decides.
				let _ = stdin.write_all(input_json.as_bytes());
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

		let status = child
			.wait_timeout(Duration::from_secs(self.timeout_secs))
			.map_err(|e| CheckError::environment(format!("lost checker '{script}': {e}")))?;
		let Some(status) = status else {
			let _ = child.kill();
			let _ = child.wait();
			return Err(CheckError::student(format!(
				"checker '{script}' timed out after {}s",
				self.timeout_secs
			)));
		};

		let stdout = stdout.join().unwrap_or_default();
		if !status.success() && stdout.trim().is_empty() {
			let stderr = stderr.join().unwrap_or_default();
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
