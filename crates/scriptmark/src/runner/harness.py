"""ScriptMark's Python harness: runs one execution unit and reports what happened.

It reports observations, never verdicts about who is at fault: the grader derives
those from which call was running, what kind of code it runs, and how the process
ended. Every piece of student code — loading the module, finding a function, reading
an attribute, serialising a value — runs inside `call()`, under its timer and import
guard, so a failure there is that call's outcome and never a harness crash.

The nonce framing protects the records from accidental output only; a student
determined to reach harness state from inside the same process could, which is why
nothing here decides anything.
"""

import sys

# The unit's working directory holds the student's file, and `python -c` puts it first
# on sys.path: a submission named json.py must not become the harness's json.
sys.path[:] = [p for p in sys.path if p]

import builtins  # noqa: E402
import importlib.machinery  # noqa: E402
import importlib.util  # noqa: E402
import inspect  # noqa: E402
import io  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import os  # noqa: E402
import runpy  # noqa: E402
import signal  # noqa: E402
import time  # noqa: E402

with open(sys.argv[1], encoding="utf-8") as _fh:
	PAYLOAD = json.load(_fh)
os.unlink(sys.argv[1])

_PREFIX = f"\n@@scriptmark:{PAYLOAD['nonce']}@@ "
# `replace`: a lone surrogate a student printed or returned must not crash the channel.
_channel = os.fdopen(os.dup(1), "w", encoding="utf-8", errors="replace")
# Between calls, stdout is a sink: a student's thread printing then must not share a pipe
# with the records. Only a deliberate write to fd 1 still reaches it.
_quiet = open(os.devnull, "w", encoding="utf-8")
sys.stdout = _quiet
_real_stdin = sys.stdin
# Student code may import helpers staged beside it — after the stdlib, so a submission
# named json.py or heapq.py never shadows what the harness or a teacher module imports.
sys.path.append(os.getcwd())
STUDENT = PAYLOAD.get("subject", "student") == "student"
STDOUT_LIMIT = 64 * 1024
FILE_LIMIT = 1024 * 1024
VALUE_LIMIT = 4 * 1024 * 1024
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
	# `__import__('os')` called directly passes no globals: look at the caller instead.
	scope = globals if globals is not None else sys._getframe(1).f_globals
	if scope.get(STUDENT_MARK) and level == 0:
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


class _CappedBytes(io.BytesIO):
	"""Keeps at most STDOUT_LIMIT bytes and remembers whether it dropped any."""

	truncated = False

	def write(self, b):
		room = STDOUT_LIMIT - self.tell()
		if len(b) > room:
			self.truncated = True
		if room > 0:
			super().write(bytes(b[:room]))
		return len(b)

	def close(self):
		"""A student's wrapper around `sys.stdout.buffer` closes it when it is collected:
		what was written must survive that."""


def capture():
	"""A stdout that behaves like a real one — `buffer`, `reconfigure`, `print(flush=True)` —
	and the bytes behind it, kept apart so a detached or closed wrapper cannot lose them."""
	raw = _CappedBytes()
	return io.TextIOWrapper(raw, encoding="utf-8", errors="backslashreplace", write_through=True), raw


def captured(stream, raw):
	try:
		stream.flush()
	except (ValueError, OSError):  # detached or closed by the student
		pass
	return raw.getvalue().decode("utf-8", "replace"), raw.truncated


# --- The one value serialiser ----------------------------------------------------------
class Unserialisable(Exception):
	pass


def to_json(value, depth=0, seen=None):
	"""What the grader sees of a Python value. Total: it never raises for a value's sake."""
	if isinstance(value, bool) or value is None or isinstance(value, str):
		return value
	if isinstance(value, int):
		if -(2**63) <= value < 2**64:
			return int(value)
		# Past what the grader's JSON can hold exactly: tag it, exactly and with its sign, so
		# it is never mistaken for a number and a record always parses. Decimal where Python
		# will print it, hexadecimal past its 4300-digit limit.
		try:
			return {"$bigint": int.__repr__(value)}
		except ValueError:
			return {"$bigint": hex(value)}
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
			key = k if isinstance(k, str) else text_of(k)
			if key in out:
				raise Unserialisable(f"two keys both read as '{key}'")
			out[key] = to_json(v, depth + 1, seen)
		return out
	return text_of(value)


def text_of(value):
	"""`str(value)`, or the type's name when the value's own `__str__` fails."""
	try:
		text = str(value)
	except BaseException:  # never under a live timer: this is harness bookkeeping
		return f"<{type(value).__name__}>"
	return text if isinstance(text, str) else f"<{type(value).__name__}>"


def error_of(exc):
	info = {
		"type": type(exc).__name__,
		"types": [cls.__name__ for cls in type(exc).__mro__],
		"message": text_of(exc),
	}
	if isinstance(exc, SystemExit):
		# The exit status CPython would report: `exit()`, `exit(0)` and `exit(False)` succeed;
		# any other code, `exit(0.0)` included, fails.
		try:
			code = exc.code
			info["clean"] = code is None or (type(code) in (int, bool) and int(code) == 0)
		except BaseException:
			info["clean"] = False
	return info


def stdin_stream(text):
	return io.TextIOWrapper(io.BytesIO((text or "").encode("utf-8")), encoding="utf-8")


class Missing(Exception):
	"""The call's target does not exist."""


class Unresolved(Exception):
	"""A `$ref` or `object` whose value was never produced in this unit."""


def call(fn, timeout, stdin=None, guarded=True, serialise=True):
	"""Run one call — its lookup, its body and its serialisation — under its timer and guard.

	Returns (outcome, stdout, truncated, elapsed_ms, live_value)."""
	out, raw = capture()
	sys.stdout = out
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
			if sys.stdout is not out:
				# The student replaced stdout with their own wrapper: flush it in their scope.
				sys.stdout.flush()
			outcome = {"returned": {"value": wire(value) if serialise else None, "type": type(value).__name__}}
		finally:
			if _HAS_TIMER:
				signal.setitimer(signal.ITIMER_REAL, 0)
	except CallTimeout:
		outcome = {"timeout": {}}
	except Missing as exc:
		outcome = {"missing": {"message": str(exc)}}
	except Unresolved as exc:
		outcome = {"unresolved": {"name": str(exc)}}
	except Unserialisable as exc:
		outcome = {"unserialisable": {"type": "Unserialisable", "message": str(exc)}}
	except BaseException as exc:  # SystemExit and KeyboardInterrupt are the code's own too
		outcome = {"raised": error_of(exc)}
	finally:
		builtins.__import__ = _original_import
		sys.stdout = _quiet
		sys.stdin = _real_stdin
	if _fired[0]:
		# The timer went off even if the code swallowed it with a bare `except:`.
		outcome = {"timeout": {}}
	elapsed = int((time.perf_counter() - start) * 1000)
	stdout, truncated = captured(out, raw)
	return outcome, stdout, truncated, elapsed, value


def wire(value):
	"""The value as a record carries it. One too large to report is replaced by its size:
	the call still returned, and a function checker still judges the live value."""
	value = to_json(value)
	size = len(json.dumps(value, ensure_ascii=False))
	if size > VALUE_LIMIT:
		return {"$too_large": f"{size} bytes of JSON"}
	return value


# --- Teacher modules -------------------------------------------------------------------
def _removed_checker(*args, **kwargs):
	raise RuntimeError(
		"@checker no longer binds a checker to a function: put "
		'check = { function = "<checker name>" } on each case it should judge '
		"(docs/test-bundles.md, 'Migrating an older spec')"
	)


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
	"""Returns (exports, duplicates): a name two modules export differently is refused."""
	exports, owner, duplicates = {}, {}, {}
	builtins.checker = _removed_checker
	try:
		for index, path in enumerate(PAYLOAD.get("imports", [])):
			directory = os.path.dirname(path)
			if directory not in sys.path:
				# After the stdlib: a helper beside the module must not shadow it.
				sys.path.append(directory)
			spec = importlib.util.spec_from_file_location(f"teacher_mod_{index}", path)
			module = importlib.util.module_from_spec(spec)
			sys.modules[spec.name] = module
			spec.loader.exec_module(module)
			for name, value in exports_of(module).items():
				if name in exports and not _same(exports[name], value):
					duplicates[name] = [owner[name], path]
				exports[name] = value
				owner.setdefault(name, path)
	finally:
		del builtins.checker
	return exports, duplicates


def _same(a, b):
	"""One object, or two equal ones: two modules that both define `TOL = 1e-6` agree."""
	if a is b:
		return True
	try:
		return bool(a == b)
	except BaseException:
		return False


def params_of(fn):
	"""Each parameter's name, kind, and whether it has a default."""
	try:
		parameters = inspect.signature(fn).parameters.values()
	except (TypeError, ValueError):
		return None
	return [{"name": p.name, "kind": p.kind.name.lower(), "default": p.default is not p.empty} for p in parameters]


# --- Calls -----------------------------------------------------------------------------
context = {}


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
		positional = [p for p in params or [] if p["name"] != "self" and "var" not in p["kind"]]
		if params is not None and len(positional) == argc:
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


def member(obj, object_id, name, what):
	"""`getattr(obj, name)`, read once. A name that does not exist is Missing; a property
	that raises — AttributeError included — is the student's exception."""
	try:
		return getattr(obj, name)
	except AttributeError:
		try:
			inspect.getattr_static(obj, name)
		except AttributeError:
			if not hasattr(type(obj), "__getattr__"):
				raise Missing(f"'{object_id}' has no {what} '{name}'") from None
		raise


def invoke(plan, student, resolved):
	"""The call a plan names. Runs inside `call()`: the lookup is student code too."""
	kind, spec = next(iter(plan["target"].items()))
	args = resolve(plan.get("args", []))
	if kind == "function":
		if PAYLOAD.get("lookup") == "exact":
			if not hasattr(student, spec["name"]):
				raise Missing(f"function '{spec['name']}' not found")
			fn, resolved[0] = getattr(student, spec["name"]), spec["name"]
		else:
			fn, resolved[0] = fuzzy_lookup(student, spec["name"], len(args))
		return fn(*args)
	if kind == "method":
		fn = member(live(spec["object"]), spec["object"], spec["name"], "method")
		if not callable(fn):
			raise Missing(f"'{spec['object']}.{spec['name']}' is not a method")
		return fn(*args)
	if kind == "attribute":
		return member(live(spec["object"]), spec["object"], spec["name"], "attribute")
	fn = context.get(spec["name"])
	if fn is None or not callable(fn):
		raise Missing(f"teacher function '{spec['name']}' not found")
	return fn(*args)


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
		name = param["name"]
		if "var" in param["kind"]:
			continue
		if name == "stdout":
			deps[name] = stdout
		elif name in context:
			deps[name] = context[name]
		elif not param["default"]:
			# prepare saw this name in scope: it is an id whose producing call failed.
			return {"unresolved": {"name": name}}
	outcome, _, _, _, returned = call(
		lambda: fn(value, check.get("expected"), **deps), timeout, guarded=False, serialise=False
	)
	if "returned" in outcome:
		return verdict_of(returned)
	if "timeout" in outcome:
		return {"error": {"type": "Timeout", "types": ["Timeout"], "message": f"checker timed out after {timeout}s"}}
	raised = outcome.get("raised") or {"type": "Error", "types": [], "message": str(outcome)}
	if "AssertionError" in raised["types"]:
		return {"rejected": {"message": raised["message"]}}
	return {"error": raised}


def observe_files(paths):
	seen = {}
	for path in paths:
		try:
			with open(path, encoding="utf-8", errors="replace") as fh:
				seen[path] = fh.read(FILE_LIMIT)
		except (OSError, ValueError):  # absent, a directory, unreadable: not the file asked for
			seen[path] = None
	return seen


def run_calls(phase, calls, student):
	"""Run setup calls or steps in order. Returns False when a setup call failed."""
	setup = phase == "setup"
	for index, plan in enumerate(calls):
		requested = next(iter(plan["target"].values()))["name"]
		resolved = [requested]
		outcome, stdout, truncated, elapsed, value = call(
			lambda: invoke(plan, student, resolved),
			plan["timeout"],
			plan.get("stdin"),
			guarded=STUDENT and "teacher" not in plan["target"],
			serialise=not setup,  # setup values are never judged, only passed on
		)
		record = {
			"kind": "call",
			"phase": phase,
			"index": index,
			"target": {"requested": requested, "resolved": resolved[0]},
			"outcome": outcome,
			"stdout": stdout,
			"stdout_truncated": truncated,
			"elapsed_ms": elapsed,
		}
		if plan.get("files"):
			record["files"] = observe_files(plan["files"])
		emit(record)
		returned = "returned" in outcome
		if returned and plan.get("id"):
			context[plan["id"]] = value
		if returned and plan.get("check"):
			verdict = run_check(plan["check"], value, stdout, plan["timeout"])
			emit({"kind": "check", "index": index, **verdict})
		if setup and not returned:
			return False
	return True


def compiles(path):
	"""Compile the student's file in memory — writing no __pycache__ beside it — and record a
	syntax error as the load call's outcome."""
	try:
		with open(path, "rb") as fh:
			compile(fh.read(), path, "exec")
		return True
	except (SyntaxError, ValueError, RecursionError, MemoryError) as exc:
		emit({"kind": "call", "phase": "load", "index": 0,
			"outcome": {"raised": {"type": "SyntaxError", "types": ["SyntaxError"], "message": text_of(exc)}},
			"stdout": "", "stdout_truncated": False, "elapsed_ms": 0})
		return False


def load_student():
	path = PAYLOAD["student"]
	if not compiles(path):
		return None
	module_holder = []

	def load():
		# An explicit loader: `Lab5.PY` is Python too, whatever its extension.
		loader = importlib.machinery.SourceFileLoader("student_mod", path)
		spec = importlib.util.spec_from_loader("student_mod", loader)
		module = importlib.util.module_from_spec(spec)
		if STUDENT:
			setattr(module, STUDENT_MARK, True)
		for key, val in PAYLOAD.get("vars", {}).items():
			setattr(module, key, val)
		sys.modules["student_mod"] = module
		loader.exec_module(module)
		module_holder.append(module)

	original_input = builtins.input
	builtins.input = lambda *a, **k: "0"
	try:
		outcome, stdout, truncated, elapsed, _ = call(
			load, PAYLOAD["load_timeout"], guarded=STUDENT, serialise=False
		)
	finally:
		builtins.input = original_input
	emit({"kind": "call", "phase": "load", "index": 0, "outcome": outcome,
		"stdout": stdout, "stdout_truncated": truncated, "elapsed_ms": elapsed})
	return module_holder[0] if "returned" in outcome else None


def run_script():
	script, path = PAYLOAD["script"], PAYLOAD["student"]
	if not compiles(path):
		return
	init = {STUDENT_MARK: True, **PAYLOAD.get("vars", {})}
	sys.argv = [path]  # as `python student.py` would see it, not the harness's payload
	outcome, stdout, truncated, elapsed, _ = call(
		lambda: runpy.run_path(path, init_globals=init, run_name="__main__"),
		script["timeout"],
		stdin=False,
		serialise=False,
	)
	raised = outcome.get("raised")
	if raised and raised["type"] == "SystemExit" and raised.get("clean"):
		outcome = {"returned": {"value": None, "type": "NoneType"}}
	record = {"kind": "call", "phase": "step", "index": 0, "outcome": outcome,
		"stdout": stdout, "stdout_truncated": truncated, "elapsed_ms": elapsed}
	if script.get("files"):
		record["files"] = observe_files(script["files"])
	emit(record)


def main():
	try:
		exports, duplicates = load_teacher_modules()
	except BaseException as exc:
		emit({"kind": "fatal", "stage": "teacher_import", "error": error_of(exc)})
		finish()
	if PAYLOAD["mode"] == "inspect":
		emit({"kind": "inspect", "duplicates": duplicates, "exports": {
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
