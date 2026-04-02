use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::{CheckInput, CheckOutput, Checker};

/// Checker that runs a Python verification script.
///
/// Protocol:
/// - stdin:  JSON `{"result": ..., "expected": ..., "context": {...}}`
/// - stdout: JSON `{"pass": true/false, "message": "..."}`
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

	pub fn with_timeout(mut self, secs: u64) -> Self {
		self.timeout_secs = secs;
		self
	}
}

impl Checker for PythonChecker {
	fn check(&self, input: &CheckInput) -> CheckOutput {
		let input_json = match serde_json::to_string(input) {
			Ok(j) => j,
			Err(e) => {
				return CheckOutput {
					pass: false,
					message: format!("Failed to serialize checker input: {e}"),
				};
			}
		};

		let mut child = match Command::new(&self.python_cmd)
			.arg(&self.script_path)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
		{
			Ok(c) => c,
			Err(e) => {
				return CheckOutput {
					pass: false,
					message: format!(
						"Failed to spawn Python checker '{}': {e}",
						self.script_path.display()
					),
				};
			}
		};

		// Write input to stdin
		if let Some(mut stdin) = child.stdin.take() {
			let _ = stdin.write_all(input_json.as_bytes());
		}

		// Wait with timeout
		let result = child.wait_timeout(Duration::from_secs(self.timeout_secs));

		match result {
			Ok(Some(status)) => {
				let stdout = {
					use std::io::Read;
					let mut buf = String::new();
					if let Some(mut out) = child.stdout.take() {
						let _ = out.read_to_string(&mut buf);
					}
					buf
				};

				if !status.success() && stdout.trim().is_empty() {
					let stderr = {
						use std::io::Read;
						let mut buf = String::new();
						if let Some(mut err) = child.stderr.take() {
							let _ = err.read_to_string(&mut buf);
						}
						buf
					};
					return CheckOutput {
						pass: false,
						message: format!(
							"Python checker '{}' exited with {}: {}",
							self.script_path.display(),
							status,
							stderr.trim()
						),
					};
				}

				// Parse JSON output
				match serde_json::from_str::<CheckOutput>(stdout.trim()) {
					Ok(output) => output,
					Err(e) => CheckOutput {
						pass: false,
						message: format!(
							"Failed to parse checker output: {e}\nRaw output: {}",
							stdout.trim()
						),
					},
				}
			}
			Ok(None) => {
				// Timeout — kill the process
				let _ = child.kill();
				CheckOutput {
					pass: false,
					message: format!(
						"Python checker '{}' timed out after {}s",
						self.script_path.display(),
						self.timeout_secs
					),
				}
			}
			Err(e) => CheckOutput {
				pass: false,
				message: format!("Error waiting for Python checker: {e}"),
			},
		}
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
		let output = checker.check(&CheckInput {
			result: json!(42),
			expected: json!(null),
			context: json!({}),
		});
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
		let output = checker.check(&CheckInput {
			result: json!(0),
			expected: json!(null),
			context: json!({}),
		});
		assert!(!output.pass);
		assert_eq!(output.message, "custom failure message");
	}

	#[test]
	fn test_python_checker_script_error() {
		let dir = tempfile::tempdir().unwrap();
		let script =
			write_checker_script(dir.path(), "check_error.py", "raise Exception('boom')\n");

		let checker = PythonChecker::new(&script);
		let output = checker.check(&CheckInput {
			result: json!(1),
			expected: json!(null),
			context: json!({}),
		});
		assert!(!output.pass);
		assert!(output.message.contains("exited with"));
	}

	#[test]
	fn test_python_checker_missing_script() {
		let checker = PythonChecker::new("/nonexistent/checker.py");
		let output = checker.check(&CheckInput {
			result: json!(1),
			expected: json!(null),
			context: json!({}),
		});
		assert!(!output.pass);
		assert!(
			output.message.contains("Failed to spawn") || output.message.contains("exited with")
		);
	}
}
