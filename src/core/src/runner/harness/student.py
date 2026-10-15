"""The student side of a unit: the student's module, and the calls the teacher side asks for.

Nothing here decides anything. It answers one request at a time: what a call did, what it
printed, which files it left, and — when the teacher side needs it — the value it made, in
the transfer encoding. Every piece of student code — loading the module, finding a
function, reading an attribute, serialising a value, reading a property for a snapshot —
runs inside `call()`, under its timer and import guard, so a failure there is that call's
outcome and never a harness crash.

What this process sends is the student's own evidence, and the teacher side treats it as
such: a student who writes to the pipe directly can only misreport their own calls.
"""

import importlib.machinery  # noqa: E402
import importlib.util  # noqa: E402
import runpy  # noqa: E402
from typing import TYPE_CHECKING  # noqa: E402

if TYPE_CHECKING:
	from matching import FunctionRules, MatchDecision, function_decision

_IN, _OUT = channel(0, 1)
# What the teacher side may send in one frame: its own values, never the student's.
REQUEST_LIMIT = 1 << 30
INIT = receive(_IN, REQUEST_LIMIT)
configure(INIT["limits"])
REFERENCE = INIT["role"] == "reference"


def reply(message):
	send(_OUT, message)


# --- The import guard -----------------------------------------------------------------
# A teaching rule, not a security boundary: the operating system's sandbox is that. It looks
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
) | frozenset(name.split(".")[0] for name in INIT.get("allowed_imports", []))
STUDENT_MARK = "__scriptmark_student__"
_original_import = builtins.__import__


def _guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
	# `__import__('os')` called directly passes no globals: look at the caller instead.
	scope = globals if globals is not None else sys._getframe(1).f_globals
	if scope.get(STUDENT_MARK) and level == 0:
		if name.split(".")[0] not in ALLOWED:
			raise ImportError(f"Module '{name}' is not allowed in student code")
	return _original_import(name, globals, locals, fromlist, level)


GUARD = None if REFERENCE else _guarded_import


# --- Values held here -----------------------------------------------------------------
# What the student's calls made, by id. Values the teacher side holds arrive as copies.
context = dict(INIT.get("vars", {}))


def live(name):
	if name not in context:
		raise Unresolved(name)
	return context[name]


def received(tree):
	"""A value the teacher side sent, rebuilt here: an id names a value made on this side."""
	return decode(tree, live)


def matched_function(
	module: object, name: str, evidence: list[MatchDecision | None]
) -> tuple[object, str]:
	policy: FunctionRules = INIT.get("functions", {"aliases": {}, "overrides": {}})
	# Read attributes only once: dynamic exports/properties belong to the guarded call.
	exports: dict[str, object] = dict(vars(module))
	absent: object = object()
	for candidate in {
		name,
		*policy.get("aliases", {}).get(name, []),
		*([policy["overrides"][name]] if name in policy.get("overrides", {}) else []),
	}:
		if candidate not in exports:
			value: object = getattr(module, candidate, absent)
			if value is not absent:
				exports[candidate] = value
	decision: MatchDecision = function_decision(
		[candidate for candidate, obj in exports.items() if callable(obj)], name, policy
	)
	evidence[0] = decision
	selected: str | None = decision["selected"]
	if selected is not None:
		return exports[selected], selected
	if (
		decision["state"] != "missing"
		or name in policy.get("overrides", {})
		or name in policy.get("aliases", {})
	):
		raise MatchingRequired(decision)
	raise Missing(f"function '{name}' not found")


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


def observe_files(paths: list[str]) -> dict[str, str | None]:
	seen: dict[str, str | None] = {}
	for path in paths:
		try:
			with open(path, encoding="utf-8") as fh:
				content: str = fh.read(FILE_LIMIT + 1)
				seen[path] = content if len(content) <= FILE_LIMIT else None
		except (OSError, ValueError):  # absent, oversized or invalid UTF-8 is no text expectation
			seen[path] = None
	return seen


# --- Requests -------------------------------------------------------------------------
module = [None]


def compiles(path):
	"""Compile the file in memory — writing no __pycache__ beside it — and answer with a
	syntax error as the load call's outcome."""
	try:
		with open(path, "rb") as fh:
			compile(fh.read(), path, "exec")
		return True
	except (SyntaxError, ValueError, RecursionError, MemoryError) as exc:
		reply({
			"compiles": False,
			"outcome": {"raised": {"type": "SyntaxError", "types": ["SyntaxError"], "message": text_of(exc)}},
			"stdout": "",
			"stdout_truncated": False,
			"elapsed_ms": 0,
		})
		return False


def load(request):
	path = INIT["subject"]
	if not compiles(path):
		return

	def run():
		# An explicit loader: `Lab5.PY` is Python too, whatever its extension.
		loader = importlib.machinery.SourceFileLoader("student_mod", path)
		spec = importlib.util.spec_from_loader("student_mod", loader)
		made = importlib.util.module_from_spec(spec)
		if not REFERENCE:
			setattr(made, STUDENT_MARK, True)
		for key, val in INIT.get("vars", {}).items():
			setattr(made, key, val)
		sys.modules["student_mod"] = made
		loader.exec_module(made)
		module[0] = made

	original_input = builtins.input
	builtins.input = lambda *a, **k: "0"
	try:
		done = call(run, request["timeout"], guard=GUARD, serialise=False)
	finally:
		builtins.input = original_input
	reply(done.record())


def feed_stdin(text):
	"""Descriptor 0 becomes a file holding `text`, as `python script.py < input` would see
	it: `input()`, `sys.stdin.buffer` and `os.read(0, …)` all read the same bytes. A file in
	the unit's own temporary directory, not a pipe and a thread to fill it: the process
	limit counts threads, and a busy machine may have none to spare."""
	import tempfile

	with tempfile.TemporaryFile() as held:
		held.write(text.encode("utf-8"))
		held.flush()
		held.seek(0)
		os.dup2(held.fileno(), 0)


def script(request):
	path = INIT["subject"]
	if not compiles(path):
		return
	init = {STUDENT_MARK: True, **INIT.get("vars", {})}
	sys.argv = [path]  # as `python student.py` would see it, not the harness's
	feed_stdin(request.get("stdin") or "")
	done = call(
		lambda: runpy.run_path(path, init_globals=init, run_name="__main__"),
		request["timeout"],
		stdin=False,
		guard=GUARD,
		serialise=False,
	)
	record = done.record()
	raised = done.outcome.get("raised")
	if raised and raised["type"] == "SystemExit" and raised.get("clean"):
		record["outcome"] = {"returned": {"value": None, "type": "NoneType"}}
	if request.get("files"):
		record["files"] = observe_files(request["files"])
	reply(record)


def invoke(request, resolved, evidence):
	"""The call a request names. Runs inside `call()`: the lookup is student code too."""
	kind, spec = next(iter(request["target"].items()))
	args = [received(a) for a in request.get("args", [])]
	if kind == "function":
		student = module[0]
		if REFERENCE:
			if not hasattr(student, spec["name"]):
				raise Missing(f"function '{spec['name']}' not found")
			fn, resolved[0] = getattr(student, spec["name"]), spec["name"]
		else:
			fn, resolved[0] = matched_function(student, spec["name"], evidence)
		return fn(*args)
	obj = received(request["object"]) if "object" in request else live(spec["object"])
	if kind == "method":
		fn = member(obj, spec["object"], spec["name"], "method")
		if not callable(fn):
			raise Missing(f"'{spec['object']}.{spec['name']}' is not a method")
		return fn(*args)
	return member(obj, spec["object"], spec["name"], "attribute")


def run_call(request):
	requested = next(iter(request["target"].values()))["name"]
	resolved = [requested]
	evidence: list[MatchDecision | None] = [None]
	done = call(
		lambda: invoke(request, resolved, evidence),
		request["timeout"],
		request.get("stdin"),
		guard=GUARD,
		serialise=request.get("serialise", True),
		then=transfer if request.get("transfer") else None,
	)
	if "returned" in done.outcome and request.get("id"):
		context[request["id"]] = done.value
	record = done.record()
	record["target"] = {"requested": requested, "resolved": resolved[0], "matching": evidence[0]}
	if request.get("files"):
		record["files"] = observe_files(request["files"])
	if done.extra is not None and done.extra != done.outcome["returned"]["value"]:
		# Sent only when the record's value would not rebuild the same thing.
		record["transfer"] = done.extra
	reply(record)


def fetch(request):
	"""A value made here, for teacher code to take by value: read now, under the timer,
	since a snapshot runs the student's properties."""
	if request["name"] not in context:
		return reply({"unresolved": request["name"]})
	value = context[request["name"]]
	done = call(lambda: transfer(value), request["timeout"], guard=GUARD, serialise=False)
	if "returned" in done.outcome:
		return reply({"value": done.value})
	reply({"outcome": done.outcome})


def canary_holds():
	"""The sandbox's own check: the grader left a file outside everything this side may read.
	Reading it means the sandbox is not there."""
	path = INIT.get("canary")
	if not path:
		return True
	try:
		with open(path, "rb"):
			return False
	except OSError:
		return True


HANDLERS = {"load": load, "script": script, "call": run_call, "fetch": fetch}


def serve():
	if not canary_holds():
		reply({"fatal": problem("Sandbox", "the student sandbox did not hold: its canary was readable")})
		os._exit(0)
	for path in INIT.get("shared", []):
		# The student's own copy of each shared module, importable as a module of its own.
		name = os.path.splitext(os.path.basename(path))[0]
		spec = importlib.util.spec_from_file_location(name, path)
		shared = importlib.util.module_from_spec(spec)
		sys.modules[name] = shared
		spec.loader.exec_module(shared)
		share(shared)
	# Student code may import helpers staged beside it: after the stdlib, so a submission
	# never stands in for what the harness imports.
	sys.path.append(os.getcwd())
	reply({"ready": True})
	while True:
		try:
			request = receive(_IN, REQUEST_LIMIT)
		except (Closed, OSError):
			break
		op = request.get("op")
		if op == "exit":
			break
		HANDLERS[op](request)
	# Threads a student left running must not hold the unit open.
	os._exit(0)


try:
	serve()
except BaseException as exc:  # a harness bug, not the student's code
	try:
		reply({"fatal": error_of(exc)})
	finally:
		os._exit(70)
