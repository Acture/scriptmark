use std::path::Path;
use std::process::Output;
use std::time::Instant;

use scriptmark_core::models::{
	CaseResult, FailureDetail, StudentFile, TestCase, TestSpec, TestStatus,
};
use tokio::process::Command;

use crate::checker::builtin::{ExactChecker, resolve_builtin};
use crate::checker::{CheckInput, Checker};
use crate::sandbox::apply_sandbox;

enum SpawnError {
	Spawn(std::io::Error),
	Timeout,
}

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

async fn read_pipe<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>) -> Vec<u8> {
	use tokio::io::AsyncReadExt;
	match pipe {
		Some(mut p) => {
			let mut buf = Vec::new();
			let _ = p.read_to_end(&mut buf).await;
			buf
		}
		None => Vec::new(),
	}
}

/// Python helper script embedded in the binary.
///
/// Takes a JSON payload on argv[1] with `file`, `function`, `args`.
/// Outputs JSON on stdout: `{"ok": true, "value": ..., "type": "..."}` or
/// `{"ok": false, "error_type": "...", "error_message": "..."}`.
const HELPER_SCRIPT: &str = r#"
import importlib.util, sys, json, builtins, io, py_compile

payload = json.loads(sys.argv[1])
file_path = payload["file"]
func_name = payload["function"]
args = payload["args"]

# Import guard: allowlist = safe stdlib + teacher-specified extras
# Hardcoded because sys.stdlib_module_names is unavailable under env_clear()
_SM_ALLOWED = frozenset({
    # core
    'builtins','sys','io','abc','enum','types','typing','dataclasses',
    'collections','functools','itertools','operator','copy','copyreg',
    # math/science
    'math','cmath','decimal','fractions','numbers','random','statistics',
    # strings/text
    'string','re','difflib','textwrap','unicodedata','codecs','encodings',
    # data formats
    'json','csv','struct','base64','binascii','hashlib','hmac',
    # time
    'time','datetime','calendar','zoneinfo',
    # data structures
    'array','heapq','bisect','queue','graphlib',
    # functional
    'contextlib','contextvars',
    # introspection
    'inspect','dis','token','tokenize','ast','symtable',
    # error handling
    'traceback','warnings','errno','faulthandler',
    # internals needed by Python itself
    'importlib','_thread','__future__','_collections_abc','_abc',
    '_functools','_operator','_io','_json','_string','_struct',
    '_sre','sre_compile','sre_constants','sre_parse','_locale',
    '_frozen_importlib','_frozen_importlib_external','_imp',
    '_warnings','_weakref','_weakrefset','_threading_local',
    '_codecs','_signal','_stat','_opcode','_typing',
    'genericpath','posixpath','ntpath','stat','keyword',
    'reprlib','pprint','linecache',
    # testing (students may use unittest/doctest)
    'unittest','doctest',
    # filesystem (pathlib is safe — open() is a builtin anyway)
    'pathlib','fnmatch','glob','tempfile',
    # misc safe
    'secrets','uuid','ipaddress','colorsys',
    'timeit','py_compile','compileall',
    'gzip','bz2','lzma','zipfile','tarfile',
    'xml','html','email',
    'logging','argparse','getopt','configparser','tomllib',
    'threading','concurrent',
}) | set(payload.get("allowed_imports", []))
_sm_orig_import = builtins.__import__
def _sm_import_guard(name, *a, **kw):
    top = name.split('.')[0]
    if top not in _SM_ALLOWED:
        raise ImportError(f"Module '{name}' is not allowed in student code")
    return _sm_orig_import(name, *a, **kw)
builtins.__import__ = _sm_import_guard

# Step 0: Syntax pre-check — catch SyntaxError/IndentationError early
try:
    py_compile.compile(file_path, doraise=True)
except py_compile.PyCompileError as e:
    print(json.dumps({"ok": False, "error_type": "SyntaxError", "error_message": str(e)}))
    sys.exit(0)

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

def _fuzzy_lookup(module, name, expected_argc=None):
    """Find function by exact name or best fuzzy match (name + signature similarity)."""
    if hasattr(module, name):
        return getattr(module, name), name
    from difflib import SequenceMatcher
    candidates = [(n, getattr(module, n)) for n in dir(module)
                  if callable(getattr(module, n)) and not n.startswith("_")]
    if not candidates:
        return None, name
    scored = []
    for n, obj in candidates:
        name_sim = SequenceMatcher(None, name.lower(), n.lower()).ratio()
        sig_bonus = 0.0
        if expected_argc is not None:
            try:
                import inspect
                params = [p for p in inspect.signature(obj).parameters.values()
                          if p.name != "self"]
                if len(params) == expected_argc:
                    sig_bonus = 0.2
            except (ValueError, TypeError):
                pass
        scored.append((n, name_sim + sig_bonus, obj))
    scored.sort(key=lambda x: -x[1])
    best_name, best_score, best_obj = scored[0]
    if best_score >= 0.5:
        return best_obj, best_name
    return None, name

func, matched_name = _fuzzy_lookup(mod, func_name, len(args))
if func is None:
    _real_print(json.dumps({"ok": False, "error_type": "AttributeError", "error_message": f"Function '{func_name}' not found"}))
    sys.exit(0)
try:
    result = func(*args)
    _real_print(json.dumps({"ok": True, "value": result, "type": type(result).__name__}))
except Exception as e:
    _real_print(json.dumps({"ok": False, "error_type": type(e).__name__, "error_message": str(e)}))
"#;

/// Chain helper script — runs teacher imports + setup + all cases in a single process.
///
/// Payload: `{file, imports, vars, setup, cases, copy_refs}`.
/// Returns JSON array of per-case results, or a setup-failure object.
///
/// Features:
/// - Teacher `@checker` decorator: auto-discovered, dependency-injected from `_ctx`
/// - `$ref` resolution in Python (live objects, no JSON round-trip)
/// - `copy_refs` (default true): deepcopy `$ref` args per case to prevent mutation
const CHAIN_HELPER_SCRIPT: &str = r#"
import importlib.util, sys, json, builtins, io, py_compile, inspect, copy

payload = json.loads(sys.argv[1])
file_path = payload["file"]
copy_refs = payload.get("copy_refs", True)
_ctx = {}
_checkers = {}  # target_function_name -> checker_fn

_real_print = builtins.print
_real_stdout = sys.stdout

# --- @checker decorator (injected into builtins for teacher modules) ---
def checker(func_or_name=None):
    if callable(func_or_name):
        name = func_or_name.__name__
        target = name[6:] if name.startswith("check_") else name
        _checkers[target] = func_or_name
        return func_or_name
    def wrap(func):
        _checkers[func_or_name] = func
        return func
    return wrap

builtins.checker = checker

def _resolve_refs(obj, do_copy=False):
    """Recursively replace '$NAME' strings with live objects from _ctx."""
    if isinstance(obj, str) and obj.startswith("$"):
        key = obj[1:]
        if key in _ctx:
            val = _ctx[key]
            return copy.deepcopy(val) if do_copy else val
        return obj
    if isinstance(obj, list):
        return [_resolve_refs(item, do_copy) for item in obj]
    if isinstance(obj, dict):
        return {k: _resolve_refs(v, do_copy) for k, v in obj.items()}
    return obj

def _make_serializable(val):
    """Best-effort conversion for JSON serialization."""
    if isinstance(val, (str, int, float, bool, type(None))):
        return val
    if isinstance(val, (list, tuple)):
        return [_make_serializable(v) for v in val]
    if isinstance(val, set):
        return sorted(_make_serializable(v) for v in val)
    if isinstance(val, dict):
        return {str(k): _make_serializable(v) for k, v in val.items()}
    return str(val)

def _call_checker(check_fn, result, expected):
    """Call checker with auto-injected _ctx dependencies based on param names."""
    sig = inspect.signature(check_fn)
    params = list(sig.parameters.keys())
    kwargs = {}
    for name in params[2:]:  # skip result, expected
        if name in _ctx:
            kwargs[name] = _ctx[name]
        else:
            return False, f"Checker dependency '{name}' not found in context"
    return check_fn(result, expected, **kwargs)

def _fail(id, etype, emsg):
    sys.stdout = _real_stdout
    _real_print(json.dumps({"setup_failed": True, "id": id,
        "error_type": etype, "error_message": emsg}))
    sys.exit(0)

# 1. Load teacher imports into _ctx
for imp_path in payload.get("imports", []):
    imp_spec = importlib.util.spec_from_file_location("teacher_mod", imp_path)
    if imp_spec is None or imp_spec.loader is None:
        _fail(imp_path, "ImportError", f"Cannot load teacher module: {imp_path}")
    tmod = importlib.util.module_from_spec(imp_spec)
    try:
        imp_spec.loader.exec_module(tmod)
    except Exception as e:
        _fail(imp_path, type(e).__name__, str(e))
    for name in dir(tmod):
        if not name.startswith("_"):
            _ctx[name] = getattr(tmod, name)

# 2. Vars (override on collision)
_ctx.update(payload.get("vars", {}))

# Import guard: allowlist = safe stdlib + teacher-specified extras
# Hardcoded because sys.stdlib_module_names is unavailable under env_clear()
_SM_ALLOWED = frozenset({
    # core
    'builtins','sys','io','abc','enum','types','typing','dataclasses',
    'collections','functools','itertools','operator','copy','copyreg',
    # math/science
    'math','cmath','decimal','fractions','numbers','random','statistics',
    # strings/text
    'string','re','difflib','textwrap','unicodedata','codecs','encodings',
    # data formats
    'json','csv','struct','base64','binascii','hashlib','hmac',
    # time
    'time','datetime','calendar','zoneinfo',
    # data structures
    'array','heapq','bisect','queue','graphlib',
    # functional
    'contextlib','contextvars',
    # introspection
    'inspect','dis','token','tokenize','ast','symtable',
    # error handling
    'traceback','warnings','errno','faulthandler',
    # internals needed by Python itself
    'importlib','_thread','__future__','_collections_abc','_abc',
    '_functools','_operator','_io','_json','_string','_struct',
    '_sre','sre_compile','sre_constants','sre_parse','_locale',
    '_frozen_importlib','_frozen_importlib_external','_imp',
    '_warnings','_weakref','_weakrefset','_threading_local',
    '_codecs','_signal','_stat','_opcode','_typing',
    'genericpath','posixpath','ntpath','stat','keyword',
    'reprlib','pprint','linecache',
    # testing (students may use unittest/doctest)
    'unittest','doctest',
    # filesystem (pathlib is safe — open() is a builtin anyway)
    'pathlib','fnmatch','glob','tempfile',
    # misc safe
    'secrets','uuid','ipaddress','colorsys',
    'timeit','py_compile','compileall',
    'gzip','bz2','lzma','zipfile','tarfile',
    'xml','html','email',
    'logging','argparse','getopt','configparser','tomllib',
    'threading','concurrent',
}) | set(payload.get("allowed_imports", []))
_sm_orig_import = builtins.__import__
def _sm_import_guard(name, *a, **kw):
    top = name.split('.')[0]
    if top not in _SM_ALLOWED:
        raise ImportError(f"Module '{name}' is not allowed in student code")
    return _sm_orig_import(name, *a, **kw)
builtins.__import__ = _sm_import_guard

# 3. Syntax-check student file
try:
    py_compile.compile(file_path, doraise=True)
except py_compile.PyCompileError as e:
    _fail("__syntax__", "SyntaxError", str(e))

# 4. Load student module (suppress input/print)
builtins.input = lambda *a, **k: "0"
sys.stdout = io.StringIO()

stu_spec = importlib.util.spec_from_file_location("student_mod", file_path)
if stu_spec is None or stu_spec.loader is None:
    _fail("__import__", "ImportError", f"Cannot load {file_path}")
student_mod = importlib.util.module_from_spec(stu_spec)
for key, val in payload.get("vars", {}).items():
    setattr(student_mod, key, val)
try:
    stu_spec.loader.exec_module(student_mod)
except Exception as e:
    _fail("__import__", type(e).__name__, str(e))
sys.stdout = _real_stdout
builtins.print = _real_print
builtins.__import__ = _sm_orig_import  # restore for teacher checkers / setup

def _fuzzy_lookup(module, name, expected_argc=None):
    """Find function by exact name or best fuzzy match (name + signature similarity)."""
    if hasattr(module, name):
        return getattr(module, name), name
    from difflib import SequenceMatcher
    candidates = [(n, getattr(module, n)) for n in dir(module)
                  if callable(getattr(module, n)) and not n.startswith("_")]
    if not candidates:
        return None, name
    scored = []
    for n, obj in candidates:
        name_sim = SequenceMatcher(None, name.lower(), n.lower()).ratio()
        sig_bonus = 0.0
        if expected_argc is not None:
            try:
                import inspect
                params = [p for p in inspect.signature(obj).parameters.values()
                          if p.name != "self"]
                if len(params) == expected_argc:
                    sig_bonus = 0.2
            except (ValueError, TypeError):
                pass
        scored.append((n, name_sim + sig_bonus, obj))
    scored.sort(key=lambda x: -x[1])
    best_name, best_score, best_obj = scored[0]
    if best_score >= 0.5:
        return best_obj, best_name
    return None, name

# 5. Run setup steps
for step in payload.get("setup", []):
    fname = step["function"]
    argc = len(step.get("args", []))
    func, matched = _fuzzy_lookup(student_mod, fname, argc)
    if func is None:
        _fail(step["id"], "AttributeError", f"Function '{fname}' not found")
    args = _resolve_refs(step.get("args", []))
    try:
        _ctx[step["id"]] = func(*args)
    except Exception as e:
        _fail(step["id"], type(e).__name__, str(e))

# 6. Run cases
results = []
for case in payload["cases"]:
    fname = case["function"]
    argc = len(case.get("args", []))
    func, matched = _fuzzy_lookup(student_mod, fname, argc)
    if func is None:
        results.append({"ok": False, "name": case["name"],
            "error_type": "AttributeError", "error_message": f"Function '{fname}' not found"})
        continue
    args = _resolve_refs(case.get("args", []), do_copy=copy_refs)
    try:
        val = func(*args)
    except Exception as e:
        results.append({"ok": False, "name": case["name"],
            "error_type": type(e).__name__, "error_message": str(e)})
        continue

    # In-process checker: explicit check_function > @checker decorator
    check_fn = None
    cfn_name = case.get("check_function")
    if cfn_name:
        check_fn = _checkers.get(cfn_name) or _ctx.get(cfn_name)
    elif fname in _checkers:
        check_fn = _checkers[fname]

    if check_fn:
        try:
            passed, msg = _call_checker(check_fn, val, case.get("expected"))
            results.append({"ok": bool(passed), "name": case["name"],
                "value": _make_serializable(val), "type": type(val).__name__,
                "checked": True, "message": msg or ""})
        except Exception as e:
            results.append({"ok": False, "name": case["name"],
                "value": _make_serializable(val), "type": type(val).__name__,
                "checked": True, "message": f"Checker error: {type(e).__name__}: {e}"})
    else:
        results.append({"ok": True, "name": case["name"],
            "value": _make_serializable(val), "type": type(val).__name__})

_real_print(json.dumps(results))
"#;

/// Executor for Python student code.
pub struct PythonExecutor {
	python_cmd: String,
	sandbox: crate::sandbox::SandboxConfig,
}

impl PythonExecutor {
	pub fn new() -> Self {
		Self {
			python_cmd: resolve_python_path("python3"),
			sandbox: crate::sandbox::SandboxConfig::default(),
		}
	}

	pub fn with_python_cmd(python_cmd: impl Into<String>) -> Self {
		let cmd: String = python_cmd.into();
		Self {
			python_cmd: resolve_python_path(&cmd),
			sandbox: crate::sandbox::SandboxConfig::default(),
		}
	}

	pub fn python_cmd(&self) -> &str {
		&self.python_cmd
	}

	/// Build a sandboxed Command with env isolation and resource limits.
	fn sandboxed_cmd(&self) -> Command {
		let mut cmd = Command::new(&self.python_cmd);
		cmd.env_clear()
			.env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
			.env("HOME", "/tmp")
			.env("PYTHONDONTWRITEBYTECODE", "1")
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped());
		#[cfg(unix)]
		apply_sandbox(&mut cmd, &self.sandbox);
		cmd
	}

	/// Spawn a command with timeout, killing the child on timeout.
	async fn spawn_with_timeout(
		&self,
		mut cmd: Command,
		timeout_secs: u64,
	) -> Result<Output, SpawnError> {
		let mut child = cmd.spawn().map_err(SpawnError::Spawn)?;

		// Take pipes and read concurrently with wait to avoid deadlock
		// (if child fills the pipe buffer, it blocks until someone reads)
		let stdout_pipe = child.stdout.take();
		let stderr_pipe = child.stderr.take();

		let timeout = std::time::Duration::from_secs(timeout_secs);
		match tokio::time::timeout(timeout, async {
			let (stdout, stderr, status) =
				tokio::join!(read_pipe(stdout_pipe), read_pipe(stderr_pipe), child.wait(),);
			status.map(|s| Output {
				status: s,
				stdout,
				stderr,
			})
		})
		.await
		{
			Ok(Ok(output)) => Ok(output),
			Ok(Err(e)) => Err(SpawnError::Spawn(e)),
			Err(_) => {
				let _ = child.kill().await;
				Err(SpawnError::Timeout)
			}
		}
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

		scored.sort_by(|a, b| b.1.cmp(&a.1));

		scored.first().map(|(f, _)| *f)
	}

	/// Execute a function-call test case via the helper script.
	async fn execute_function_call(
		&self,
		student_file: &StudentFile,
		function_name: &str,
		case: &TestCase,
		vars: &std::collections::HashMap<String, serde_json::Value>,
		allowed_imports: &[String],
		timeout_secs: u64,
	) -> CaseResult {
		let start = Instant::now();

		let payload = serde_json::json!({
			"file": student_file.path.to_string_lossy(),
			"function": function_name,
			"args": case.args,
			"vars": vars,
			"allowed_imports": allowed_imports,
		});

		let mut cmd = self.sandboxed_cmd();
		cmd.arg("-c").arg(HELPER_SCRIPT).arg(payload.to_string());

		let result = self.spawn_with_timeout(cmd, timeout_secs).await;
		let elapsed = start.elapsed().as_millis() as u64;

		match result {
			Err(SpawnError::Timeout) => CaseResult {
				case_name: case.name.clone(),
				status: TestStatus::Timeout,
				actual: None,
				expected: case.expect.as_ref().map(|v| v.to_string()),
				failure: Some(FailureDetail {
					message: format!("Timed out after {timeout_secs}s"),
					details: String::new(),
				}),
				elapsed_ms: Some(elapsed),
			},
			Err(SpawnError::Spawn(e)) => CaseResult {
				case_name: case.name.clone(),
				status: TestStatus::Error,
				actual: None,
				expected: None,
				failure: Some(FailureDetail {
					message: format!("Failed to spawn python: {e}"),
					details: String::new(),
				}),
				elapsed_ms: Some(elapsed),
			},
			Ok(output) => {
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

		let mut cmd = self.sandboxed_cmd();
		cmd.arg(&student_file.path);

		if let Some(stdin_data) = &case.stdin {
			cmd.stdin(std::process::Stdio::piped());
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
	/// Kills the child on timeout to prevent orphaned processes.
	async fn await_child_output(
		&self,
		mut child: tokio::process::Child,
		case: &TestCase,
		timeout_secs: u64,
		start: Instant,
	) -> CaseResult {
		// Take pipes and read concurrently with wait (same pattern as spawn_with_timeout)
		let stdout_pipe = child.stdout.take();
		let stderr_pipe = child.stderr.take();

		let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
			let (stdout, _stderr, status) =
				tokio::join!(read_pipe(stdout_pipe), read_pipe(stderr_pipe), child.wait(),);
			status.map(|s| Output {
				status: s,
				stdout,
				stderr: _stderr,
			})
		})
		.await;

		let elapsed = start.elapsed().as_millis() as u64;

		match result {
			Err(_) => {
				let _ = child.kill().await;
				CaseResult {
					case_name: case.name.clone(),
					status: TestStatus::Timeout,
					actual: None,
					expected: case.expected_stdout.clone(),
					failure: Some(FailureDetail {
						message: format!("Timed out after {timeout_secs}s"),
						details: String::new(),
					}),
					elapsed_ms: Some(elapsed),
				}
			}
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

	/// Execute all cases in chain mode (single Python subprocess).
	///
	/// Teacher imports + vars + setup + cases all run in one process.
	/// `$ref` resolution happens Python-side with live objects.
	pub async fn execute_chain(
		&self,
		student_files: &[StudentFile],
		spec: &TestSpec,
		cases: &[TestCase],
		timeout_secs: u64,
	) -> Vec<CaseResult> {
		let start = Instant::now();

		// Collect function hints from cases for smarter file matching
		let func_hint = cases
			.iter()
			.find_map(|c| c.function.as_deref())
			.or(spec.meta.function.as_deref());

		let student_file =
			match self.find_student_file_with_hint(student_files, &spec.meta.file, func_hint) {
				Some(f) => f,
				None => {
					return cases
						.iter()
						.map(|c| CaseResult {
							case_name: c.name.clone(),
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
						})
						.collect();
				}
			};

		// Build case payloads — include check_function if specified via CheckSpec
		let case_payloads: Vec<serde_json::Value> = cases
			.iter()
			.map(|c| {
				let func_name = c
					.function
					.as_deref()
					.or(spec.meta.function.as_deref())
					.unwrap_or("__missing__");

				let check_function = c.check.as_ref().and_then(|ch| {
					if let scriptmark_core::models::spec::CheckMethod::Detailed(spec) = ch {
						spec.function.clone()
					} else {
						None
					}
				});

				let mut obj = serde_json::json!({
					"name": c.name,
					"function": func_name,
					"args": c.args,
				});
				if let Some(expected) = &c.expect {
					obj["expected"] = expected.clone();
				}
				if let Some(cf) = check_function {
					obj["check_function"] = serde_json::Value::String(cf);
				}
				obj
			})
			.collect();

		// Build setup payloads
		let setup_payloads: Vec<serde_json::Value> = spec
			.setup
			.iter()
			.filter_map(|s| {
				s.function.as_ref().map(|f| {
					serde_json::json!({
						"id": s.id,
						"function": f,
						"args": s.args,
					})
				})
			})
			.collect();

		let payload = serde_json::json!({
			"file": student_file.path.to_string_lossy(),
			"imports": spec.meta.imports,
			"vars": spec.vars,
			"setup": setup_payloads,
			"cases": case_payloads,
			"copy_refs": spec.meta.copy_refs,
			"allowed_imports": spec.meta.allowed_imports,
		});

		let mut cmd = self.sandboxed_cmd();
		cmd.arg("-c")
			.arg(CHAIN_HELPER_SCRIPT)
			.arg(payload.to_string());

		let result = self.spawn_with_timeout(cmd, timeout_secs).await;
		let elapsed = start.elapsed().as_millis() as u64;

		match result {
			Err(SpawnError::Timeout) => cases
				.iter()
				.map(|c| CaseResult {
					case_name: c.name.clone(),
					status: TestStatus::Timeout,
					actual: None,
					expected: c.expect.as_ref().map(|v| v.to_string()),
					failure: Some(FailureDetail {
						message: format!("Chain timed out after {timeout_secs}s"),
						details: String::new(),
					}),
					elapsed_ms: Some(elapsed),
				})
				.collect(),
			Err(SpawnError::Spawn(e)) => cases
				.iter()
				.map(|c| CaseResult {
					case_name: c.name.clone(),
					status: TestStatus::Error,
					actual: None,
					expected: None,
					failure: Some(FailureDetail {
						message: format!("Failed to spawn python: {e}"),
						details: String::new(),
					}),
					elapsed_ms: Some(elapsed),
				})
				.collect(),
			Ok(output) => {
				let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
				self.parse_chain_results(&stdout, cases, elapsed)
			}
		}
	}

	/// Parse JSON output from the chain helper script.
	fn parse_chain_results(
		&self,
		stdout: &str,
		cases: &[TestCase],
		elapsed_ms: u64,
	) -> Vec<CaseResult> {
		let parsed: serde_json::Value = match serde_json::from_str(stdout) {
			Ok(v) => v,
			Err(e) => {
				return cases
					.iter()
					.map(|c| CaseResult {
						case_name: c.name.clone(),
						status: TestStatus::Error,
						actual: Some(stdout.to_string()),
						expected: None,
						failure: Some(FailureDetail {
							message: format!("Failed to parse chain output: {e}"),
							details: stdout.to_string(),
						}),
						elapsed_ms: Some(elapsed_ms),
					})
					.collect();
			}
		};

		// Check for setup failure
		if let Some(true) = parsed.get("setup_failed").and_then(|v| v.as_bool()) {
			let id = parsed
				.get("id")
				.and_then(|v| v.as_str())
				.unwrap_or("unknown");
			let error_type = parsed
				.get("error_type")
				.and_then(|v| v.as_str())
				.unwrap_or("Error");
			let error_message = parsed
				.get("error_message")
				.and_then(|v| v.as_str())
				.unwrap_or("");
			return cases
				.iter()
				.map(|c| CaseResult {
					case_name: c.name.clone(),
					status: TestStatus::Error,
					actual: None,
					expected: None,
					failure: Some(FailureDetail {
						message: format!("Setup '{id}' failed: {error_type}: {error_message}"),
						details: String::new(),
					}),
					elapsed_ms: Some(elapsed_ms),
				})
				.collect();
		}

		// Parse array of case results
		let results_array = match parsed.as_array() {
			Some(arr) => arr,
			None => {
				return cases
					.iter()
					.map(|c| CaseResult {
						case_name: c.name.clone(),
						status: TestStatus::Error,
						actual: Some(stdout.to_string()),
						expected: None,
						failure: Some(FailureDetail {
							message: "Chain output is not an array".to_string(),
							details: stdout.to_string(),
						}),
						elapsed_ms: Some(elapsed_ms),
					})
					.collect();
			}
		};

		cases
			.iter()
			.enumerate()
			.map(|(i, case)| {
				let entry = match results_array.get(i) {
					Some(e) => e,
					None => {
						return CaseResult {
							case_name: case.name.clone(),
							status: TestStatus::Error,
							actual: None,
							expected: None,
							failure: Some(FailureDetail {
								message: "No result returned for this case".to_string(),
								details: String::new(),
							}),
							elapsed_ms: Some(elapsed_ms),
						};
					}
				};

				// If Python-side checker already evaluated, use its verdict
				if entry
					.get("checked")
					.and_then(|v| v.as_bool())
					.unwrap_or(false)
				{
					let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
					let msg = entry
						.get("message")
						.and_then(|v| v.as_str())
						.unwrap_or("")
						.to_string();
					let actual = entry.get("value").map(|v| v.to_string());
					if ok {
						return CaseResult {
							case_name: case.name.clone(),
							status: TestStatus::Passed,
							actual,
							expected: case.expect.as_ref().map(|v| v.to_string()),
							failure: None,
							elapsed_ms: Some(elapsed_ms),
						};
					} else {
						return CaseResult {
							case_name: case.name.clone(),
							status: TestStatus::Failed,
							actual,
							expected: case.expect.as_ref().map(|v| v.to_string()),
							failure: Some(FailureDetail {
								message: msg,
								details: String::new(),
							}),
							elapsed_ms: Some(elapsed_ms),
						};
					}
				}

				// Otherwise, evaluate using Rust-side checker (same as single mode)
				let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
				if !ok {
					// Build a fake stdout for evaluate_function_result
					let json_str = entry.to_string();
					return self.evaluate_function_result(&json_str, case, elapsed_ms);
				}

				let actual_value = entry
					.get("value")
					.cloned()
					.unwrap_or(serde_json::Value::Null);

				// Check expect_error — function succeeded but we expected error
				if case.expect_error.is_some() {
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
			})
			.collect()
	}

	/// Execute a single test case. Dispatches to function-call or IO mode.
	pub async fn execute_case(
		&self,
		student_files: &[StudentFile],
		spec: &TestSpec,
		case: &TestCase,
		timeout_secs: u64,
	) -> CaseResult {
		let student_file = match self.find_student_file_with_hint(
			student_files,
			&spec.meta.file,
			spec.meta.function.as_deref(),
		) {
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
			self.execute_function_call(
				student_file,
				function_name,
				case,
				&spec.vars,
				&spec.meta.allowed_imports,
				timeout_secs,
			)
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
