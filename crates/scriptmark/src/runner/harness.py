"""ScriptMark's Python harness: runs one execution unit and reports what happened.

It reports observations, never verdicts about who is at fault: the grader derives
those from which call was running, what kind of code it runs, and how the process
ended. The nonce framing protects the records from accidental output only; a student
determined to reach harness state from inside the same process could, which is why
nothing here decides anything.
"""

import builtins
import importlib.util
import inspect
import io
import json
import math
import os
import py_compile
import runpy
import signal
import sys
import time

with open(sys.argv[1], encoding="utf-8") as _fh:
	PAYLOAD = json.load(_fh)
os.unlink(sys.argv[1])

_PREFIX = f"\n@@scriptmark:{PAYLOAD['nonce']}@@ "
_channel = os.fdopen(os.dup(1), "w", encoding="utf-8")
_real_stdout = sys.stdout
_real_stdin = sys.stdin
STDOUT_LIMIT = 64 * 1024
FILE_LIMIT = 1024 * 1024
DEPTH_LIMIT = 100


def emit(record):
	_channel.write(_PREFIX + json.dumps(record, ensure_ascii=False) + "\n")
	_channel.flush()


def finish():
	"""Stop now: threads a student left running must not hold the unit open."""
	emit({"kind": "done"})
	_channel.close()
	os._exit(0)


# --- The import guard -----------------------------------------------------------------
# A policy check, not a security boundary: importlib.import_module bypasses it. It looks
# at where the import statement runs, so an allowlisted stdlib module importing its own
# internals is never mistaken for the student importing them.
ALLOWED = frozenset(
	{
		# core
		"builtins", "sys", "io", "abc", "enum", "types", "typing", "dataclasses",
		"collections", "functools", "itertools", "operator", "copy", "copyreg",
		# math/science
		"math", "cmath", "decimal", "fractions", "numbers", "random", "statistics",
		# strings/text
		"string", "re", "difflib", "textwrap", "unicodedata", "codecs", "encodings",
		# data formats
		"json", "csv", "struct", "base64", "binascii", "hashlib", "hmac",
		# time
		"time", "datetime", "calendar", "zoneinfo",
		# data structures
		"array", "heapq", "bisect", "queue", "graphlib",
		# functional
		"contextlib", "contextvars",
		# introspection
		"inspect", "dis", "token", "tokenize", "ast", "symtable",
		# error handling
		"traceback", "warnings", "errno", "faulthandler",
		# internals needed by Python itself
		"importlib", "_thread", "__future__", "_collections_abc", "_abc",
		"_functools", "_operator", "_io", "_json", "_string", "_struct",
		"_sre", "sre_compile", "sre_constants", "sre_parse", "_locale",
		"_frozen_importlib", "_frozen_importlib_external", "_imp",
		"_warnings", "_weakref", "_weakrefset", "_threading_local",
		"_codecs", "_signal", "_stat", "_opcode", "_typing",
		"genericpath", "posixpath", "ntpath", "stat", "keyword",
		"reprlib", "pprint", "linecache",
		# testing (students may use unittest/doctest)
		"unittest", "doctest",
		# filesystem (pathlib is safe — open() is a builtin anyway)
		"pathlib", "fnmatch", "glob", "tempfile",
		# misc safe
		"secrets", "uuid", "ipaddress", "colorsys", "timeit", "py_compile", "compileall",
		"gzip", "bz2", "lzma", "zipfile", "tarfile", "xml", "html", "email",
		"logging", "argparse", "getopt", "configparser", "tomllib", "threading", "concurrent",
	}
) | frozenset(PAYLOAD.get("allowed_imports", []))
STUDENT_MARK = "__scriptmark_student__"
_original_import = builtins.__import__


def _guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
	if globals is not None and globals.get(STUDENT_MARK) and level == 0:
		if name.split(".")[0] not in ALLOWED:
			raise ImportError(f"Module '{name}' is not allowed in student code")
	return _original_import(name, globals, locals, fromlist, level)


# --- Timers ---------------------------------------------------------------------------
class CallTimeout(BaseException):
	"""Raised by SIGALRM. A BaseException, so `except Exception` cannot swallow it."""


_fired = [False]


def _on_alarm(signum, frame):
	_fired[0] = True
	raise CallTimeout()


_HAS_TIMER = hasattr(signal, "setitimer")
if _HAS_TIMER:
	signal.signal(signal.SIGALRM, _on_alarm)


class Capture(io.TextIOBase):
	"""A stdout replacement that keeps at most STDOUT_LIMIT characters."""

	encoding = "utf-8"

	def __init__(self):
		self._parts = []
		self._size = 0
		self.truncated = False

	def writable(self):
		return True

	def write(self, s):
		if not isinstance(s, str):
			raise TypeError(f"write() argument must be str, not {type(s).__name__}")
		room = STDOUT_LIMIT - self._size
		if room > 0:
			self._parts.append(s[:room])
			self._size += min(len(s), room)
		if len(s) > room:
			self.truncated = True
		return len(s)

	def getvalue(self):
		return "".join(self._parts)


# --- The one value serialiser ----------------------------------------------------------
class Unserialisable(Exception):
	pass


def to_json(value, depth=0, seen=None):
	"""What the grader sees of a Python value. Runs inside the student call's scope."""
	if value is None or isinstance(value, (bool, int, str)):
		return value
	if isinstance(value, float):
		return value if math.isfinite(value) else repr(value)
	if depth >= DEPTH_LIMIT:
		return "<too deep>"
	seen = seen or set()
	if id(value) in seen:
		return "<cycle>"
	if isinstance(value, (list, tuple, set, frozenset, dict)):
		seen = seen | {id(value)}
	if isinstance(value, (list, tuple)):
		return [to_json(v, depth + 1, seen) for v in value]
	if isinstance(value, (set, frozenset)):
		items = [to_json(v, depth + 1, seen) for v in value]
		try:
			return sorted(items)
		except TypeError:  # mixed types: any stable order will do
			return sorted(items, key=lambda v: json.dumps(v, sort_keys=True))
	if isinstance(value, dict):
		out = {}
		for k, v in value.items():
			key = k if isinstance(k, str) else str(k)
			if key in out:
				raise Unserialisable(f"two keys both read as '{key}'")
			out[key] = to_json(v, depth + 1, seen)
		return out
	try:
		text = str(value)
	except Exception:
		return f"<{type(value).__name__}>"
	return text if isinstance(text, str) else f"<{type(value).__name__}>"


def error_of(exc):
	return {
		"type": type(exc).__name__,
		"types": [cls.__name__ for cls in type(exc).__mro__],
		"message": str(exc),
	}


def stdin_stream(text):
	return io.TextIOWrapper(io.BytesIO((text or "").encode("utf-8")), encoding="utf-8")


def call(fn, timeout, stdin=None, guarded=True, serialise=True):
	"""Run one call. Returns (outcome, stdout, truncated, elapsed_ms, live_value)."""
	capture = Capture()
	sys.stdout = capture
	sys.stdin = stdin_stream(stdin) if stdin is not False else _real_stdin
	if guarded:
		builtins.__import__ = _guarded_import
	_fired[0] = False
	start = time.perf_counter()
	value = None
	try:
		if _HAS_TIMER and timeout:
			signal.setitimer(signal.ITIMER_REAL, timeout, 0.05)
		try:
			value = fn()
			outcome = {"returned": {"value": to_json(value) if serialise else None, "type": type(value).__name__}}
		finally:
			if _HAS_TIMER:
				signal.setitimer(signal.ITIMER_REAL, 0)
	except CallTimeout:
		outcome = {"timeout": {}}
	except Unserialisable as exc:
		outcome = {"unserialisable": {"type": type(exc).__name__, "message": str(exc)}}
	except BaseException as exc:  # SystemExit and KeyboardInterrupt are the code's own too
		outcome = {"raised": error_of(exc)}
	finally:
		builtins.__import__ = _original_import
		sys.stdout = _real_stdout
		sys.stdin = _real_stdin
	if _fired[0]:
		# The timer went off even if the code swallowed it with a bare `except:`.
		outcome = {"timeout": {}}
	elapsed = int((time.perf_counter() - start) * 1000)
	return outcome, capture.getvalue(), capture.truncated, elapsed, value


# --- Teacher modules -------------------------------------------------------------------
def exports_of(module):
	"""`__all__` when present; otherwise public names, minus modules and minus classes and
	functions the module merely imported from elsewhere."""
	names = getattr(module, "__all__", None)
	if names is None:
		names = []
		for name in dir(module):
			if name.startswith("_"):
				continue
			value = getattr(module, name)
			if inspect.ismodule(value):
				continue
			if (inspect.isclass(value) or inspect.isroutine(value)) and getattr(
				value, "__module__", None
			) != module.__name__:
				continue
			names.append(name)
	return {name: getattr(module, name) for name in names}


def load_teacher_modules():
	exports = {}
	for index, path in enumerate(PAYLOAD.get("imports", [])):
		directory = os.path.dirname(path)
		if directory not in sys.path:
			sys.path.insert(0, directory)
		spec = importlib.util.spec_from_file_location(f"teacher_mod_{index}", path)
		module = importlib.util.module_from_spec(spec)
		sys.modules[spec.name] = module
		spec.loader.exec_module(module)
		exports.update(exports_of(module))
	return exports


def params_of(fn):
	try:
		return [p.name for p in inspect.signature(fn).parameters.values()]
	except (TypeError, ValueError):
		return None


# --- Calls -----------------------------------------------------------------------------
context = {}


class Unresolved(Exception):
	"""A `$ref` or `object` whose value was never produced in this unit."""


class Missing(Exception):
	"""The call's target does not exist."""


def resolve(value):
	if isinstance(value, str) and value.startswith("$"):
		if value.startswith("$$"):
			return value[1:]
		if value[1:] not in context:
			raise Unresolved(value[1:])
		return context[value[1:]]
	if isinstance(value, list):
		return [resolve(v) for v in value]
	if isinstance(value, dict):
		return {k: resolve(v) for k, v in value.items()}
	return value


def fuzzy_lookup(module, name, argc):
	"""Find a function by exact name or best fuzzy match. P-673 owns this rule."""
	if hasattr(module, name):
		return getattr(module, name), name
	from difflib import SequenceMatcher

	scored = []
	for candidate in dir(module):
		obj = getattr(module, candidate)
		if candidate.startswith("_") or not callable(obj):
			continue
		score = SequenceMatcher(None, name.lower(), candidate.lower()).ratio()
		params = params_of(obj)
		if params is not None and len([p for p in params if p != "self"]) == argc:
			score += 0.2
		scored.append((score, candidate, obj))
	scored.sort(key=lambda s: -s[0])
	if scored and scored[0][0] >= 0.5:
		return scored[0][2], scored[0][1]
	raise Missing(f"function '{name}' not found")


def live(object_id):
	if object_id not in context:
		raise Unresolved(object_id)
	return context[object_id]


def bind(target, student, args):
	"""The callable a target names, and the name it resolved to."""
	kind, spec = next(iter(target.items()))
	if kind == "function":
		if PAYLOAD.get("lookup") == "exact":
			if not hasattr(student, spec["name"]):
				raise Missing(f"function '{spec['name']}' not found")
			fn, resolved = getattr(student, spec["name"]), spec["name"]
		else:
			fn, resolved = fuzzy_lookup(student, spec["name"], len(args))
		return (lambda: fn(*args)), resolved
	if kind == "method":
		fn = getattr(live(spec["object"]), spec["name"], None)
		if fn is None or not callable(fn):
			raise Missing(f"'{spec['object']}' has no method '{spec['name']}'")
		return (lambda: fn(*args)), spec["name"]
	if kind == "attribute":
		obj = live(spec["object"])
		if not hasattr(obj, spec["name"]):
			raise Missing(f"'{spec['object']}' has no attribute '{spec['name']}'")
		return (lambda: getattr(obj, spec["name"])), spec["name"]
	fn = context.get(spec["name"])
	if fn is None or not callable(fn):
		raise Missing(f"teacher function '{spec['name']}' not found")
	return (lambda: fn(*args)), spec["name"]


def verdict_of(returned):
	if isinstance(returned, bool):
		return {"verdict": {"pass": returned, "message": ""}}
	if (
		isinstance(returned, tuple)
		and len(returned) == 2
		and isinstance(returned[0], bool)
		and isinstance(returned[1], (str, type(None)))
	):
		return {"verdict": {"pass": returned[0], "message": returned[1] or ""}}
	return {
		"error": {
			"type": "CheckerContract",
			"types": ["CheckerContract"],
			"message": f"checker returned {type(returned).__name__}; expected bool or (bool, str)",
		}
	}


def run_check(check, value, stdout, timeout):
	fn = context.get(check["function"])
	deps = {}
	for param in (params_of(fn) or [])[2:]:
		deps[param] = stdout if param == "stdout" and param not in context else context.get(param)
	outcome, _, _, _, returned = call(
		lambda: fn(value, check.get("expected"), **deps), timeout, guarded=False, serialise=False
	)
	if "returned" in outcome:
		return verdict_of(returned)
	if "timeout" in outcome:
		return {"error": {"type": "Timeout", "types": ["Timeout"], "message": f"checker timed out after {timeout}s"}}
	raised = outcome["raised"]
	if "AssertionError" in raised["types"]:
		return {"rejected": {"message": raised["message"]}}
	return {"error": raised}


def observe_files(paths):
	seen = {}
	for path in paths:
		try:
			with open(path, encoding="utf-8", errors="replace") as fh:
				seen[path] = fh.read(FILE_LIMIT)
		except (FileNotFoundError, IsADirectoryError):
			seen[path] = None
	return seen


def run_calls(phase, calls, student):
	"""Run setup calls or steps in order. Returns False when a setup call failed."""
	for index, plan in enumerate(calls):
		record = {"kind": "call", "phase": phase, "index": index}
		requested = next(iter(plan["target"].values()))["name"]
		value = None
		try:
			args = resolve(plan.get("args", []))
			fn, resolved = bind(plan["target"], student, args)
		except Unresolved as exc:
			record["outcome"] = {"unresolved": {"name": str(exc)}}
		except Missing as exc:
			record["outcome"] = {"missing": {"message": str(exc)}}
		else:
			teacher = "teacher" in plan["target"]
			outcome, stdout, truncated, elapsed, value = call(
				fn, plan["timeout"], plan.get("stdin"), guarded=not teacher
			)
			record.update(
				target={"requested": requested, "resolved": resolved},
				outcome=outcome,
				stdout=stdout,
				stdout_truncated=truncated,
				elapsed_ms=elapsed,
			)
			if plan.get("files"):
				record["files"] = observe_files(plan["files"])
		emit(record)
		returned = "returned" in record["outcome"]
		if returned and plan.get("id"):
			context[plan["id"]] = value
		if returned and plan.get("check"):
			verdict = run_check(plan["check"], value, record.get("stdout", ""), plan["timeout"])
			emit({"kind": "check", "index": index, **verdict})
		if phase == "setup" and not returned:
			return False
	return True


def load_student():
	path = PAYLOAD["student"]
	try:
		py_compile.compile(path, doraise=True)
	except py_compile.PyCompileError as exc:
		emit({"kind": "call", "phase": "load", "index": 0,
			"outcome": {"raised": {"type": "SyntaxError", "types": ["SyntaxError"], "message": str(exc)}},
			"stdout": "", "stdout_truncated": False, "elapsed_ms": 0})
		return None
	spec = importlib.util.spec_from_file_location("student_mod", path)
	module = importlib.util.module_from_spec(spec)
	setattr(module, STUDENT_MARK, True)
	for key, val in PAYLOAD.get("vars", {}).items():
		setattr(module, key, val)
	sys.modules["student_mod"] = module
	original_input = builtins.input
	builtins.input = lambda *a, **k: "0"
	try:
		outcome, stdout, truncated, elapsed, _ = call(
			lambda: spec.loader.exec_module(module), PAYLOAD["load_timeout"], serialise=False
		)
	finally:
		builtins.input = original_input
	emit({"kind": "call", "phase": "load", "index": 0, "outcome": outcome,
		"stdout": stdout, "stdout_truncated": truncated, "elapsed_ms": elapsed})
	return module if "returned" in outcome else None


def run_script():
	script = PAYLOAD["script"]
	init = {STUDENT_MARK: True, **PAYLOAD.get("vars", {})}
	outcome, stdout, truncated, elapsed, _ = call(
		lambda: runpy.run_path(PAYLOAD["student"], init_globals=init, run_name="__main__"),
		script["timeout"],
		stdin=False,
		serialise=False,
	)
	raised = outcome.get("raised")
	if raised and raised["type"] == "SystemExit" and raised["message"] in ("", "0", "None"):
		outcome = {"returned": {"value": None, "type": "NoneType"}}
	record = {"kind": "call", "phase": "step", "index": 0, "outcome": outcome,
		"stdout": stdout, "stdout_truncated": truncated, "elapsed_ms": elapsed}
	if script.get("files"):
		record["files"] = observe_files(script["files"])
	emit(record)


def main():
	try:
		exports = load_teacher_modules()
	except BaseException as exc:
		emit({"kind": "fatal", "stage": "teacher_import", "error": error_of(exc)})
		finish()
	if PAYLOAD["mode"] == "inspect":
		emit({"kind": "inspect", "exports": {
			name: {"callable": callable(value), "params": params_of(value) if callable(value) else None}
			for name, value in exports.items()
		}})
		finish()
	context.update(exports)
	context.update(PAYLOAD.get("vars", {}))
	emit({"kind": "ready"})
	if PAYLOAD.get("script"):
		run_script()
		finish()
	student = load_student()
	if student is not None and run_calls("setup", PAYLOAD.get("setup", []), student):
		run_calls("step", PAYLOAD.get("steps", []), student)
	finish()


try:
	main()
except BaseException as exc:  # a harness bug, not anybody's code under test
	emit({"kind": "fatal", "stage": "harness", "error": error_of(exc)})
	finish()
