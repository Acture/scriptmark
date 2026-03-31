use std::path::Path;
use std::time::Instant;

use scriptmark_core::models::{
    CaseResult, FailureDetail, StudentFile, TestCase, TestSpec, TestStatus,
};
use tokio::process::Command;

use crate::checker::builtin::{ExactChecker, resolve_builtin};
use crate::checker::{CheckInput, Checker};

/// Python helper script embedded in the binary.
///
/// Takes a JSON payload on argv[1] with `file`, `function`, `args`.
/// Outputs JSON on stdout: `{"ok": true, "value": ..., "type": "..."}` or
/// `{"ok": false, "error_type": "...", "error_message": "..."}`.
const HELPER_SCRIPT: &str = r#"
import importlib.util, sys, json, builtins, io

payload = json.loads(sys.argv[1])
file_path = payload["file"]
func_name = payload["function"]
args = payload["args"]

# Override input/print during module loading to prevent student top-level code
# from blocking (input) or polluting stdout (print)
_real_print = builtins.print
_real_stdout = sys.stdout
builtins.input = lambda *a, **k: "0"
sys.stdout = io.StringIO()  # swallow prints during module load

spec = importlib.util.spec_from_file_location("student_mod", file_path)
if spec is None or spec.loader is None:
    sys.stdout = _real_stdout
    _real_print(json.dumps({"ok": False, "error_type": "ImportError", "error_message": f"Cannot load {file_path}"}))
    sys.exit(0)

mod = importlib.util.module_from_spec(spec)

# Inject vars as module-level globals (teacher-defined constants)
for key, val in payload.get("vars", {}).items():
    setattr(mod, key, val)

try:
    spec.loader.exec_module(mod)
except Exception as e:
    sys.stdout = _real_stdout
    _real_print(json.dumps({"ok": False, "error_type": type(e).__name__, "error_message": str(e)}))
    sys.exit(0)

# Restore stdout for function execution — student function output is captured
sys.stdout = _real_stdout

if not hasattr(mod, func_name):
    _real_print(json.dumps({"ok": False, "error_type": "AttributeError", "error_message": f"Function '{func_name}' not found"}))
    sys.exit(0)

func = getattr(mod, func_name)
try:
    result = func(*args)
    _real_print(json.dumps({"ok": True, "value": result, "type": type(result).__name__}))
except Exception as e:
    _real_print(json.dumps({"ok": False, "error_type": type(e).__name__, "error_message": str(e)}))
"#;

/// Executor for Python student code.
pub struct PythonExecutor {
    python_cmd: String,
}

impl PythonExecutor {
    pub fn new() -> Self {
        Self {
            python_cmd: "python3".to_string(),
        }
    }

    pub fn with_python_cmd(python_cmd: impl Into<String>) -> Self {
        Self {
            python_cmd: python_cmd.into(),
        }
    }

    pub fn python_cmd(&self) -> &str {
        &self.python_cmd
    }

    /// Find the student file matching the spec's file pattern.
    fn find_student_file<'a>(
        &self,
        student_files: &'a [StudentFile],
        pattern: &str,
    ) -> Option<&'a StudentFile> {
        // Exact suffix match first
        if let Some(f) = student_files.iter().find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(pattern))
        }) {
            return Some(f);
        }
        // Stem-contains match as fallback
        let stem = Path::new(pattern)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(pattern);
        student_files.iter().find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(stem))
        })
    }

    /// Execute a function-call test case via the helper script.
    async fn execute_function_call(
        &self,
        student_file: &StudentFile,
        function_name: &str,
        case: &TestCase,
        vars: &std::collections::HashMap<String, serde_json::Value>,
        timeout_secs: u64,
    ) -> CaseResult {
        let start = Instant::now();

        let payload = serde_json::json!({
            "file": student_file.path.to_string_lossy(),
            "function": function_name,
            "args": case.args,
            "vars": vars,
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Command::new(&self.python_cmd)
                .arg("-c")
                .arg(HELPER_SCRIPT)
                .arg(payload.to_string())
                .output(),
        )
        .await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Err(_) => {
                // Timeout
                CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Timeout,
                    actual: None,
                    expected: case.expect.as_ref().map(|v| v.to_string()),
                    failure: Some(FailureDetail {
                        message: format!("Timed out after {timeout_secs}s"),
                        details: String::new(),
                    }),
                    elapsed_ms: Some(elapsed),
                }
            }
            Ok(Err(e)) => {
                // Process spawn error
                CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Error,
                    actual: None,
                    expected: None,
                    failure: Some(FailureDetail {
                        message: format!("Failed to spawn python: {e}"),
                        details: String::new(),
                    }),
                    elapsed_ms: Some(elapsed),
                }
            }
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                self.evaluate_function_result(&stdout, case, elapsed)
            }
        }
    }

    /// Parse the JSON output from the helper script and check against expectations.
    fn evaluate_function_result(
        &self,
        stdout: &str,
        case: &TestCase,
        elapsed_ms: u64,
    ) -> CaseResult {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout);

        let json = match parsed {
            Ok(v) => v,
            Err(e) => {
                return CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Error,
                    actual: Some(stdout.to_string()),
                    expected: None,
                    failure: Some(FailureDetail {
                        message: format!("Failed to parse helper output: {e}"),
                        details: stdout.to_string(),
                    }),
                    elapsed_ms: Some(elapsed_ms),
                };
            }
        };

        let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

        if !ok {
            let error_type = json
                .get("error_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let error_message = json
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Check if this was an expected error
            if let Some(expected_error) = &case.expect_error
                && error_type == expected_error
            {
                return CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Passed,
                    actual: Some(format!("{error_type}: {error_message}")),
                    expected: Some(format!("{expected_error} (expected)")),
                    failure: None,
                    elapsed_ms: Some(elapsed_ms),
                };
            }

            return CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Failed,
                actual: Some(format!("{error_type}: {error_message}")),
                expected: case
                    .expect
                    .as_ref()
                    .map(|v| v.to_string())
                    .or_else(|| case.expect_error.as_ref().map(|e| format!("{e} error"))),
                failure: Some(FailureDetail {
                    message: format!("{error_type}: {error_message}"),
                    details: String::new(),
                }),
                elapsed_ms: Some(elapsed_ms),
            };
        }

        // Student code returned successfully — but we expected an error?
        if case.expect_error.is_some() {
            let actual_value = json
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            return CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Failed,
                actual: Some(actual_value.to_string()),
                expected: case.expect_error.as_ref().map(|e| format!("{e} error")),
                failure: Some(FailureDetail {
                    message: format!(
                        "Expected {} but function returned {}",
                        case.expect_error.as_deref().unwrap_or("error"),
                        actual_value
                    ),
                    details: String::new(),
                }),
                elapsed_ms: Some(elapsed_ms),
            };
        }

        // Normal return — check the value
        let actual_value = json
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let checker = self.resolve_checker(case);
        let check_result = checker.check(&CheckInput {
            result: actual_value.clone(),
            expected: case.expect.clone().unwrap_or(serde_json::Value::Null),
            context: serde_json::Value::Null,
        });

        if check_result.pass {
            CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Passed,
                actual: Some(actual_value.to_string()),
                expected: case.expect.as_ref().map(|v| v.to_string()),
                failure: None,
                elapsed_ms: Some(elapsed_ms),
            }
        } else {
            CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Failed,
                actual: Some(actual_value.to_string()),
                expected: case.expect.as_ref().map(|v| v.to_string()),
                failure: Some(FailureDetail {
                    message: check_result.message,
                    details: String::new(),
                }),
                elapsed_ms: Some(elapsed_ms),
            }
        }
    }

    /// Execute an IO-based test case (stdin → stdout).
    async fn execute_io_test(
        &self,
        student_file: &StudentFile,
        case: &TestCase,
        timeout_secs: u64,
    ) -> CaseResult {
        let start = Instant::now();

        let mut cmd = Command::new(&self.python_cmd);
        cmd.arg(&student_file.path);

        if let Some(stdin_data) = &case.stdin {
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    return CaseResult {
                        case_name: case.name.clone(),
                        status: TestStatus::Error,
                        actual: None,
                        expected: None,
                        failure: Some(FailureDetail {
                            message: format!("Failed to spawn: {e}"),
                            details: String::new(),
                        }),
                        elapsed_ms: Some(start.elapsed().as_millis() as u64),
                    };
                }
            };

            // Write stdin then drop to close pipe
            {
                use tokio::io::AsyncWriteExt;
                if let Some(child_stdin) = child.stdin.as_mut() {
                    let _ = child_stdin.write_all(stdin_data.as_bytes()).await;
                    let _ = child_stdin.shutdown().await;
                }
            }
            child.stdin.take(); // close stdin

            self.await_child_output(child, case, timeout_secs, start)
                .await
        } else {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            let child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    return CaseResult {
                        case_name: case.name.clone(),
                        status: TestStatus::Error,
                        actual: None,
                        expected: None,
                        failure: Some(FailureDetail {
                            message: format!("Failed to spawn: {e}"),
                            details: String::new(),
                        }),
                        elapsed_ms: Some(start.elapsed().as_millis() as u64),
                    };
                }
            };

            self.await_child_output(child, case, timeout_secs, start)
                .await
        }
    }

    /// Wait for a child process with timeout, then compare stdout.
    async fn await_child_output(
        &self,
        child: tokio::process::Child,
        case: &TestCase,
        timeout_secs: u64,
        start: Instant,
    ) -> CaseResult {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Err(_) => CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Timeout,
                actual: None,
                expected: case.expected_stdout.clone(),
                failure: Some(FailureDetail {
                    message: format!("Timed out after {timeout_secs}s"),
                    details: String::new(),
                }),
                elapsed_ms: Some(elapsed),
            },
            Ok(Err(e)) => CaseResult {
                case_name: case.name.clone(),
                status: TestStatus::Error,
                actual: None,
                expected: None,
                failure: Some(FailureDetail {
                    message: format!("Process error: {e}"),
                    details: String::new(),
                }),
                elapsed_ms: Some(elapsed),
            },
            Ok(Ok(output)) => {
                let actual_stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let expected = case.expected_stdout.as_deref().unwrap_or("");

                if actual_stdout == expected {
                    CaseResult {
                        case_name: case.name.clone(),
                        status: TestStatus::Passed,
                        actual: Some(actual_stdout),
                        expected: Some(expected.to_string()),
                        failure: None,
                        elapsed_ms: Some(elapsed),
                    }
                } else {
                    CaseResult {
                        case_name: case.name.clone(),
                        status: TestStatus::Failed,
                        actual: Some(actual_stdout.clone()),
                        expected: Some(expected.to_string()),
                        failure: Some(FailureDetail {
                            message: "stdout mismatch".to_string(),
                            details: format!("expected:\n{expected}\nactual:\n{actual_stdout}"),
                        }),
                        elapsed_ms: Some(elapsed),
                    }
                }
            }
        }
    }

    /// Resolve which checker to use for a test case.
    fn resolve_checker(&self, case: &TestCase) -> Box<dyn Checker> {
        use crate::checker::python_checker::PythonChecker;
        use crate::checker::rhai_checker::RhaiChecker;
        use scriptmark_core::models::spec::CheckMethod;

        if let Some(check) = &case.check {
            match check {
                CheckMethod::Builtin(name) => {
                    if let Some(c) = resolve_builtin(name, None) {
                        return c;
                    }
                }
                CheckMethod::Detailed(spec) => {
                    if let Some(name) = &spec.builtin
                        && let Some(c) = resolve_builtin(name, spec.tolerance)
                    {
                        return c;
                    }
                    if let Some(expr) = &spec.rhai {
                        return Box::new(RhaiChecker::new(expr));
                    }
                    if let Some(script) = &spec.python {
                        return Box::new(
                            PythonChecker::new(script).with_python_cmd(self.python_cmd()),
                        );
                    }
                    // TODO: exec, wasm checkers
                }
            }
        }
        Box::new(ExactChecker)
    }

    /// Execute a single test case. Dispatches to function-call or IO mode.
    pub async fn execute_case(
        &self,
        student_files: &[StudentFile],
        spec: &TestSpec,
        case: &TestCase,
        timeout_secs: u64,
    ) -> CaseResult {
        let student_file = match self.find_student_file(student_files, &spec.meta.file) {
            Some(f) => f,
            None => {
                return CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Error,
                    actual: None,
                    expected: None,
                    failure: Some(FailureDetail {
                        message: format!(
                            "No file matching '{}' found in submission",
                            spec.meta.file
                        ),
                        details: String::new(),
                    }),
                    elapsed_ms: Some(0),
                };
            }
        };

        if let Some(function_name) = &spec.meta.function {
            self.execute_function_call(student_file, function_name, case, &spec.vars, timeout_secs)
                .await
        } else {
            self.execute_io_test(student_file, case, timeout_secs).await
        }
    }
}

impl Default for PythonExecutor {
    fn default() -> Self {
        Self::new()
    }
}
