"""What both sides of a unit share: framing, the two value encodings, and one call under
its timer.

A unit runs as two processes. The teacher side (`teacher.py`) holds the teacher's modules
and checkers, drives the plan and writes the records ScriptMark reads; the student side
(`student.py`) holds only the student's module and runs the calls the teacher side asks
for. They talk over one pair of pipes, a frame at a time: a 4-byte big-endian length, then
that many bytes of JSON. Every frame a side reads is checked against the largest frame
that side could legitimately be sent at that point, before its body is read.

Two encodings cross the pipes:

- the *record* encoding (`to_json`, `wire`) is what the grader judges and keeps as
  evidence: plain JSON, lossy where JSON is (sets sorted, tuples as lists, objects as
  their text);
- the *transfer* encoding (`encode`, `decode`) rebuilds a value on the other side for code
  that runs there: a teacher checker judging a student's value, or a student call taking a
  teacher's. It keeps tuples, sets, exact numbers, shared references and cycles. An
  instance of a class from a shared module is rebuilt from the receiving side's own copy
  of the class, without `__init__`; any other object arrives as a `Snapshot` of its
  attributes and public properties, read on the side that owns it.
"""

import sys

# The unit's working directory holds the student's file, and `python -c` puts it first on
# sys.path: a submission named json.py must not become the harness's json.
sys.path[:] = [p for p in sys.path if p]

import base64  # noqa: E402
import builtins  # noqa: E402
import datetime  # noqa: E402
import decimal  # noqa: E402
import fractions  # noqa: E402
import inspect  # noqa: E402
import io  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import os  # noqa: E402
import signal  # noqa: E402
import struct  # noqa: E402
import time  # noqa: E402

HEADER = struct.Struct(">I")
# The record limits, set from the grader's own numbers before anything runs (`configure`).
STDOUT_LIMIT = 64 * 1024
FILE_LIMIT = 1024 * 1024
VALUE_LIMIT = 4 * 1024 * 1024
MESSAGE_LIMIT = 64 * 1024
TRANSFER_LIMIT = 32 * 1024 * 1024
DEPTH_LIMIT = 100


def configure(limits):
	global STDOUT_LIMIT, FILE_LIMIT, VALUE_LIMIT, MESSAGE_LIMIT, TRANSFER_LIMIT, DEPTH_LIMIT
	STDOUT_LIMIT = limits["stdout"]
	FILE_LIMIT = limits["file"]
	VALUE_LIMIT = limits["value"]
	MESSAGE_LIMIT = limits["message"]
	TRANSFER_LIMIT = limits["transfer"]
	DEPTH_LIMIT = limits["depth"]


def channel(read_fd, write_fd):
	"""Move this process's pipe to the other side off descriptors 0 and 1, which then read
	nothing and write nowhere: no print, `sys.__stdout__` or `os.write(1, …)` reaches it."""
	read_fd, write_fd = os.dup(read_fd), os.dup(write_fd)
	null = os.open(os.devnull, os.O_RDWR)
	os.dup2(null, 0)
	os.dup2(null, 1)
	os.close(null)
	return read_fd, write_fd


# --- Framing ----------------------------------------------------------------------------
class Closed(Exception):
	"""The other side closed its end, or went away mid-frame."""


class Oversized(Exception):
	"""A frame announced more bytes than may arrive at this point."""


class Garbled(Exception):
	"""A frame that is not what this point of the protocol expects."""


def frame(obj):
	# `replace`: a lone surrogate a student printed or returned must not stop the frame.
	body = json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8", "replace")
	return HEADER.pack(len(body)) + body


def write_all(fd, data):
	view = memoryview(data)
	while view:
		view = view[os.write(fd, view) :]


def send(fd, obj):
	write_all(fd, frame(obj))


def read_exact(fd, n):
	chunks, got = [], 0
	while got < n:
		chunk = os.read(fd, min(n - got, 1 << 20))
		if not chunk:
			raise Closed()
		chunks.append(chunk)
		got += len(chunk)
	return b"".join(chunks)


def receive(fd, bound):
	"""One frame, refused before its body is read when it announces more than `bound`."""
	(size,) = HEADER.unpack(read_exact(fd, HEADER.size))
	if size > bound:
		raise Oversized(f"a frame of {size} bytes where at most {bound} may arrive")
	return parse(read_exact(fd, size))


def parse(body):
	try:
		return json.loads(body.decode("utf-8"))
	except (ValueError, RecursionError) as exc:  # bad UTF-8, bad JSON, nested past the parser
		raise Garbled(f"not a frame: {text_of(exc)}") from None


# --- Reporting ----------------------------------------------------------------------------
def text_of(value):
	"""`str(value)`, or the type's name when the value's own `__str__` fails."""
	try:
		text = str(value)
	except BaseException:  # never under a live timer: this is harness bookkeeping
		return f"<{type(value).__name__}>"
	return text if isinstance(text, str) else f"<{type(value).__name__}>"


def clip(message):
	"""A message kept to MESSAGE_LIMIT characters, saying how long it was."""
	if len(message) <= MESSAGE_LIMIT:
		return message
	return f"{message[:MESSAGE_LIMIT]}… (cut from {len(message)} characters)"


def error_of(exc):
	info = {
		"type": type(exc).__name__,
		"types": [cls.__name__ for cls in type(exc).__mro__],
		"message": clip(text_of(exc)),
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


def problem(kind, message):
	return {"type": kind, "types": [kind], "message": clip(message)}


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

	def writelines(self, lines):
		# The C method would write past the limit without calling `write`.
		for line in lines:
			self.write(line)

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
	# `io.BytesIO.write(buffer, b)` still reaches past `write`: cut here too.
	data = raw.getvalue()
	return data[:STDOUT_LIMIT].decode("utf-8", "replace"), raw.truncated or len(data) > STDOUT_LIMIT


def stdin_stream(text):
	return io.TextIOWrapper(io.BytesIO((text or "").encode("utf-8")), encoding="utf-8")


# Between calls, stdout is a sink: what a leftover thread prints is not evidence.
_quiet = open(os.devnull, "w", encoding="utf-8")
_real_stdin = sys.stdin
sys.stdout = _quiet


class Missing(Exception):
	"""The call's target does not exist."""


class Unresolved(Exception):
	"""A `$ref` or `object` whose value was never produced in this unit."""


class Unserialisable(Exception):
	pass


class MatchingRequired(Exception):
	"""Function lookup needs a teacher's decision: never the student's outcome."""

	def __init__(self, decision):
		self.decision = decision
		super().__init__(f"function matching needs review: {decision}")


class Call:
	"""One call's outcome: what `call()` observed, and the live value it returned."""

	def __init__(self, outcome, stdout, truncated, elapsed, value, extra):
		self.outcome = outcome
		self.stdout = stdout
		self.truncated = truncated
		self.elapsed = elapsed
		self.value = value
		self.extra = extra

	def record(self):
		return {
			"outcome": self.outcome,
			"stdout": self.stdout,
			"stdout_truncated": self.truncated,
			"elapsed_ms": self.elapsed,
		}


def call(fn, timeout, stdin=None, guard=None, serialise=True, then=None):
	"""Run one call — its lookup, its body, its serialisation and `then(value)` — under its
	timer and, for student code, the import `guard`.

	`then` runs inside the timed region too: it may run the code that owns the value (a
	property read for a snapshot), and what it raises is that call's outcome."""
	out, raw = capture()
	sys.stdout = out
	sys.stdin = stdin_stream(stdin) if stdin is not False else _real_stdin
	original_import = builtins.__import__
	if guard is not None:
		builtins.__import__ = guard
	_fired[0] = False
	start = time.perf_counter()
	value, extra = None, None
	try:
		if _HAS_TIMER and timeout:
			signal.setitimer(signal.ITIMER_REAL, timeout, 0.05)
		try:
			value = fn()
			if sys.stdout is not out:
				# The student replaced stdout with their own wrapper: flush it in their scope.
				sys.stdout.flush()
			outcome = {"returned": {"value": wire(value) if serialise else None, "type": type(value).__name__}}
			if then is not None:
				extra = then(value)
		finally:
			if _HAS_TIMER:
				signal.setitimer(signal.ITIMER_REAL, 0)
	except CallTimeout:
		outcome = {"timeout": {}}
	except Missing as exc:
		outcome = {"missing": {"message": str(exc)}}
	except MatchingRequired as exc:
		outcome = {"matching": {"message": str(exc), "decision": exc.decision}}
	except Unresolved as exc:
		outcome = {"unresolved": {"name": str(exc)}}
	except Unserialisable as exc:
		outcome = {"unserialisable": {"type": "Unserialisable", "message": str(exc)}}
	except BaseException as exc:  # SystemExit and KeyboardInterrupt are the code's own too
		outcome = {"raised": error_of(exc)}
	finally:
		builtins.__import__ = original_import
		sys.stdout = _quiet
		sys.stdin = _real_stdin
	if _fired[0]:
		# The timer went off even if the code swallowed it with a bare `except:`.
		outcome = {"timeout": {}}
	elapsed = int((time.perf_counter() - start) * 1000)
	stdout, truncated = captured(out, raw)
	return Call(outcome, stdout, truncated, elapsed, value, extra if "returned" in outcome else None)


# --- The record encoding --------------------------------------------------------------------
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


def utf8_size(text):
	"""The bytes `text` takes on the channel, counted a slice at a time rather than through
	a second full copy of a value that may already fill most of the memory limit."""
	step = 1 << 20
	return sum(len(text[i : i + step].encode("utf-8", "replace")) for i in range(0, len(text), step))


def json_size(tree):
	return utf8_size(json.dumps(tree, ensure_ascii=False, separators=(",", ":")))


def wire(value):
	"""The value as a record carries it. One too large to report is replaced by its size:
	the call still returned, and a function checker still judges it through the transfer
	encoding."""
	value = to_json(value)
	size = utf8_size(json.dumps(value, ensure_ascii=False))
	if size > VALUE_LIMIT:
		return {"$too_large": f"{size} bytes of JSON"}
	return value


# --- The transfer encoding --------------------------------------------------------------------
class Snapshot:
	"""An object whose class the receiving side does not have: its instance attributes and
	public properties, as read on the side that owns it. `__scriptmark_class__` names the
	class it was taken from."""

	def __init__(self, cls, attrs):
		object.__setattr__(self, "__scriptmark_class__", cls)
		for name, value in attrs.items():
			object.__setattr__(self, name, value)

	def __repr__(self):
		fields = ", ".join(
			f"{k}={v!r}" for k, v in vars(self).items() if k != "__scriptmark_class__"
		)
		return f"<{self.__scriptmark_class__} snapshot: {fields}>"


# Classes whose instances are rebuilt from the receiving side's own copy, by `module.qualname`.
SHARED = {}


def share(module):
	"""Register every class a shared module defines."""
	for value in vars(module).values():
		if inspect.isclass(value) and getattr(value, "__module__", None) == module.__name__:
			SHARED[f"{module.__name__}.{value.__qualname__}"] = value


def _shared_name(cls):
	"""The shared class `cls` is, or derives from: a student's subclass travels as its
	shared base."""
	for base in cls.__mro__:
		name = f"{getattr(base, '__module__', '')}.{getattr(base, '__qualname__', '')}"
		if SHARED.get(name) is base:
			return name
	return None


def _state(value):
	"""An instance's own attributes: its `__dict__`, and the slots it has filled."""
	state = dict(vars(value)) if hasattr(value, "__dict__") else {}
	for cls in type(value).__mro__:
		slots = cls.__dict__.get("__slots__", ())
		for slot in [slots] if isinstance(slots, str) else slots:
			if slot in ("__dict__", "__weakref__") or slot in state:
				continue
			try:
				state[slot] = getattr(value, slot)
			except AttributeError:  # declared, never set
				pass
	return state


def _properties(value):
	"""The public properties `value`'s class defines, read now: this runs the owner's code."""
	read = {}
	for cls in type(value).__mro__:
		if cls is object:
			continue
		for name, attr in cls.__dict__.items():
			if isinstance(attr, property) and not name.startswith("_") and name not in read:
				read[name] = getattr(value, name)
	return read


_ATOMS = (decimal.Decimal, fractions.Fraction, complex, datetime.date, datetime.time, datetime.timedelta, bytes, bytearray)


class _Node:
	__slots__ = ("kind", "body", "refs", "number", "building")

	def __init__(self, kind):
		self.kind = kind
		self.body = None
		self.refs = 1
		self.number = None
		self.building = False


def encode(value):
	"""The transfer encoding of `value`. Runs the owner's code (properties, `__iter__` of a
	container subclass) and raises what it raises; call it under the owner's timer."""
	nodes = {}
	keep = []  # every object visited stays alive, so no id is reused while encoding
	root = _graph(value, 0, nodes, keep)
	numbers = [0]
	return _emit(root, numbers)


def _graph(value, depth, nodes, keep):
	"""Pass one: the value graph, each container and object visited once, with how often it
	is referred to — so pass two numbers only what is actually shared."""
	if value is None or isinstance(value, (bool, str)):
		return value
	if isinstance(value, int):
		return value if -(2**63) <= value < 2**64 else ("int", int(value))
	if isinstance(value, float):
		return value if math.isfinite(value) else ("float", repr(value))
	if isinstance(value, _ATOMS):
		return ("atom", value)
	if depth >= DEPTH_LIMIT:
		return ("text", "<too deep>", type(value).__name__)
	node = nodes.get(id(value))
	if node is not None:
		node.refs += 1
		return node
	keep.append(value)
	shared = _shared_name(type(value))
	if shared is not None:
		node = nodes[id(value)] = _Node("shared")
		node.body = (shared, {k: _graph(v, depth + 1, nodes, keep) for k, v in _state(value).items()})
		return node
	if isinstance(value, (list, tuple, set, frozenset)):
		kind = "list" if isinstance(value, list) else "tuple" if isinstance(value, tuple) else "set" if isinstance(value, set) else "frozenset"
		node = nodes[id(value)] = _Node(kind)
		node.body = [_graph(v, depth + 1, nodes, keep) for v in value]
		return node
	if isinstance(value, dict):
		node = nodes[id(value)] = _Node("dict")
		node.body = [(_graph(k, depth + 1, nodes, keep), _graph(v, depth + 1, nodes, keep)) for k, v in value.items()]
		return node
	if inspect.isroutine(value) or inspect.isclass(value) or inspect.ismodule(value):
		return ("text", text_of(value), type(value).__name__)
	module = getattr(type(value), "__module__", "") or ""
	if module.split(".")[0] == "numpy" and callable(getattr(value, "tolist", None)):
		return _graph(value.tolist(), depth, nodes, keep)
	if hasattr(value, "__dict__") or any("__slots__" in cls.__dict__ for cls in type(value).__mro__[:-1]):
		node = nodes[id(value)] = _Node("object")
		attrs = {k: _graph(v, depth + 1, nodes, keep) for k, v in _state(value).items()}
		props = {k: _graph(v, depth + 1, nodes, keep) for k, v in _properties(value).items()}
		node.body = (type(value).__qualname__, attrs, props)
		return node
	return ("text", text_of(value), type(value).__name__)


def _atom(value):
	if isinstance(value, decimal.Decimal):
		return {"$": "decimal", "v": str(value)}
	if isinstance(value, fractions.Fraction):
		return {"$": "fraction", "v": [_emit_int(value.numerator), _emit_int(value.denominator)]}
	if isinstance(value, complex):
		return {"$": "complex", "v": [repr(value.real), repr(value.imag)]}
	if isinstance(value, datetime.datetime):
		return {"$": "datetime", "v": value.isoformat()}
	if isinstance(value, datetime.date):
		return {"$": "date", "v": value.isoformat()}
	if isinstance(value, datetime.time):
		return {"$": "time", "v": value.isoformat()}
	if isinstance(value, datetime.timedelta):
		return {"$": "timedelta", "v": [value.days, value.seconds, value.microseconds]}
	return {"$": "bytes" if isinstance(value, bytes) else "bytearray", "v": base64.b64encode(bytes(value)).decode("ascii")}


def _emit_int(value):
	return value if -(2**63) <= value < 2**64 else {"$": "int", "v": hex(value)}


def _emit(item, numbers):
	"""Pass two: JSON, plain wherever plain JSON says the same thing."""
	if not isinstance(item, (_Node, tuple)):
		return item
	if isinstance(item, tuple):
		kind = item[0]
		if kind == "int":
			return {"$": "int", "v": hex(item[1])}
		if kind == "float":
			return {"$": "float", "v": item[1]}
		if kind == "atom":
			return _atom(item[1])
		return {"$": "text", "v": item[1], "type": item[2]}
	if item.number is not None:
		return {"$": "ref", "#": item.number}
	kind = item.kind
	if kind in ("tuple", "frozenset"):
		# Made from their items, so they are numbered once those are: a way back into one
		# while its items are written cannot be rebuilt, and reads as a cycle, as the
		# record encoding says.
		if item.building:
			return {"$": "text", "v": "<cycle>", "type": kind}
		item.building = True
		items = [_emit(v, numbers) for v in item.body]
		item.building = False
		tagged = {}
		if item.refs > 1:
			item.number = numbers[0]
			numbers[0] += 1
			tagged["#"] = item.number
		return {"$": kind, **tagged, "v": items}
	tagged = {}
	if item.refs > 1:
		item.number = numbers[0]
		numbers[0] += 1
		tagged["#"] = item.number
	if kind == "list":
		items = [_emit(v, numbers) for v in item.body]
		return items if not tagged else {"$": "list", **tagged, "v": items}
	if kind == "set":
		return {"$": kind, **tagged, "v": [_emit(v, numbers) for v in item.body]}
	if kind == "dict":
		pairs = [(_emit(k, numbers), _emit(v, numbers)) for k, v in item.body]
		if not tagged and all(isinstance(k, str) and k != "$" for k, _ in pairs):
			return dict(pairs)
		return {"$": "dict", **tagged, "v": [[k, v] for k, v in pairs]}
	if kind == "shared":
		name, state = item.body
		return {"$": "shared", **tagged, "class": name, "state": {k: _emit(v, numbers) for k, v in state.items()}}
	cls, attrs, props = item.body
	return {
		"$": "object",
		**tagged,
		"class": cls,
		"attrs": {k: _emit(v, numbers) for k, v in attrs.items()},
		"properties": {k: _emit(v, numbers) for k, v in props.items()},
	}


def decode(tree, resolve=None):
	"""Rebuild a transfer-encoded value. `resolve(name)` answers `{"$": "id"}` references to
	values that live on this side. Raises Garbled for anything the encoding cannot mean."""
	return _decode(tree, {}, resolve, 0)


def _decode(tree, made, resolve, depth):
	if depth > DEPTH_LIMIT + 2:
		raise Garbled(f"a value nested deeper than {DEPTH_LIMIT}")
	if tree is None or isinstance(tree, (bool, int, float, str)):
		return tree
	if isinstance(tree, list):
		return [_decode(v, made, resolve, depth + 1) for v in tree]
	if not isinstance(tree, dict):
		raise Garbled(f"unexpected {type(tree).__name__}")
	kind = tree.get("$")
	if kind is None:
		return {k: _decode(v, made, resolve, depth + 1) for k, v in tree.items()}
	try:
		return _tagged(kind, tree, made, resolve, depth)
	except Garbled:
		raise
	except RecursionError:
		raise Garbled("a value nested too deeply") from None
	except Exception as exc:  # a tag whose body does not fit it
		raise Garbled(f"a '{kind}' value that cannot be rebuilt: {text_of(exc)}") from None


def _number(tree, made, obj):
	if "#" in tree:
		number = tree["#"]
		if not isinstance(number, int) or number in made:
			raise Garbled("a value numbered twice")
		made[number] = obj
	return obj


def _tagged(kind, tree, made, resolve, depth):
	body = tree.get("v")
	again = lambda v: _decode(v, made, resolve, depth + 1)  # noqa: E731
	if kind == "ref":
		if tree.get("#") not in made:
			raise Garbled("a reference to nothing before it")
		return made[tree["#"]]
	if kind == "id":
		if resolve is None:
			raise Garbled("a reference to a value this side does not hold")
		return resolve(tree["name"])
	if kind == "int":
		# Its length is the frame's, already bounded; hexadecimal parses in linear time.
		if not isinstance(body, str):
			raise Garbled("an integer that is not text")
		return int(body, 16)
	if kind == "float":
		return float(body)
	if kind == "decimal":
		return decimal.Decimal(body)
	if kind == "fraction":
		return fractions.Fraction(again(body[0]), again(body[1]))
	if kind == "complex":
		return complex(float(body[0]), float(body[1]))
	if kind == "datetime":
		return datetime.datetime.fromisoformat(body)
	if kind == "date":
		return datetime.date.fromisoformat(body)
	if kind == "time":
		return datetime.time.fromisoformat(body)
	if kind == "timedelta":
		return datetime.timedelta(days=body[0], seconds=body[1], microseconds=body[2])
	if kind in ("bytes", "bytearray"):
		data = base64.b64decode(body, validate=True)
		return data if kind == "bytes" else bytearray(data)
	if kind == "text":
		return str(body)
	if kind == "list":
		out = _number(tree, made, [])
		out.extend(again(v) for v in body)
		return out
	if kind == "set":
		out = _number(tree, made, set())
		out.update(again(v) for v in body)
		return out
	if kind in ("tuple", "frozenset"):
		# Built from their items, so a cycle back through one cannot be rebuilt.
		number = tree.get("#")
		if number is not None and number in made:
			raise Garbled("a value numbered twice")
		items = [again(v) for v in body]
		return _number(tree, made, tuple(items) if kind == "tuple" else frozenset(items))
	if kind == "dict":
		out = _number(tree, made, {})
		for key, value in body:
			out[again(key)] = again(value)
		return out
	if kind == "shared":
		cls = SHARED.get(tree["class"])
		if cls is None:
			raise Garbled(f"no shared class '{tree['class']}' on this side")
		try:
			obj = cls.__new__(cls)
		except TypeError:  # a __new__ that wants arguments: rebuilt without it, as without __init__
			obj = object.__new__(cls)
		obj = _number(tree, made, obj)
		for name, value in tree["state"].items():
			object.__setattr__(obj, name, again(value))
		return obj
	if kind == "object":
		obj = _number(tree, made, Snapshot(str(tree["class"]), {}))
		for part in ("attrs", "properties"):
			for name, value in tree[part].items():
				object.__setattr__(obj, name, again(value))
		return obj
	raise Garbled(f"an unknown kind of value '{kind}'")


def transfer(value):
	"""The transfer encoding of `value`, or a marker saying how large it would have been."""
	tree = encode(value)
	size = json_size(tree)
	if size > TRANSFER_LIMIT:
		return {"$": "too_large", "size": size}
	return tree
