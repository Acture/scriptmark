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
import tempfile  # noqa: E402
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


# One reused buffer. Past this, the body spills to temporary files: a frame's length
# travels ahead of its bytes, and a pipe cannot be rewound, but the bytes themselves must
# not sit in the process a second time. Each spill stays under the sandbox's 10 MB
# RLIMIT_FSIZE; a body past that is the next file, not a larger one.
CHUNK = 64 * 1024
SPILL = 8 * 1024 * 1024


class _Raw:
	"""JSON bytes already encoded, either still in `CHUNK` or spilled beside the process."""

	__slots__ = ("_buf", "_files", "size", "_closed")

	def __init__(self, buf, files, size):
		self._buf = buf
		self._files = files
		self.size = size
		self._closed = False

	def write_to(self, fd):
		for block in self._blocks():
			write_all(fd, block)

	def _blocks(self):
		if self._buf is not None:
			yield self._buf
			return
		for handle in self._files:
			handle.seek(0)
			while True:
				block = handle.read(CHUNK)
				if not block:
					break
				yield block
			handle.seek(0)

	def close(self):
		if self._closed:
			return
		self._closed = True
		for handle in self._files:
			handle.close()
		self._files = []
		self._buf = None


class _Capture:
	"""Where an encoder writes. `limit` counts bytes and then drops what was stored;
	`spaces` counts the extra byte `json.dumps` puts after each separator without writing it."""

	__slots__ = ("buf", "file", "files", "written", "extra", "limit", "spaces", "over")

	def __init__(self, limit, spaces):
		self.buf = bytearray()
		self.file = None
		self.files = []
		self.written = 0
		self.extra = 0
		self.limit = limit
		self.spaces = spaces
		self.over = False

	def space(self):
		if not self.spaces:
			return
		self.extra += 1
		if self.over or self.limit is None:
			return
		if self.written + self.extra > self.limit:
			self.over = True
			self._drop()

	def write(self, data):
		if not data:
			return
		n = len(data)
		if self.over:
			self.written += n
			return
		if self.limit is not None and self.written + self.extra + n > self.limit:
			self.over = True
			self._drop()
			self.written += n
			return
		self._store(data)
		self.written += n

	def write_raw(self, raw):
		n = raw.size
		if self.over:
			self.written += n
			return
		if self.limit is not None and self.written + self.extra + n > self.limit:
			self.over = True
			self._drop()
			self.written += n
			return
		for block in raw._blocks():
			self._store(block)
		self.written += n

	def _store(self, data):
		view = memoryview(data)
		while len(view):
			if self.file is None and not self.files and len(self.buf) + len(view) <= CHUNK:
				self.buf.extend(view)
				return
			if self.file is None:
				self.file = tempfile.TemporaryFile()
				if self.buf:
					self.file.write(self.buf)
					self.buf.clear()
			room = SPILL - self.file.tell()
			if room <= 0:
				self.file.flush()
				self.files.append(self.file)
				self.file = tempfile.TemporaryFile()
				continue
			self.file.write(view[:room])
			view = view[room:]

	def _drop(self):
		"""Throw the bytes away. Closing a spill the limit already refused can raise
		`EFBIG` as its buffer flushes; that must not replace the size result."""
		self.buf.clear()
		files = self.files
		self.files = []
		if self.file is not None:
			files.append(self.file)
			self.file = None
		for handle in files:
			try:
				handle.close()
			except OSError:
				pass

	def finish(self):
		"""The bytes written, or None when `limit` was passed. The count stays in `written`."""
		if self.over:
			self._drop()
			return None
		if self.file is None and not self.files:
			raw = _Raw(bytes(self.buf), [], self.written)
			self.buf = bytearray()
			return raw
		if self.file is not None:
			self.file.flush()
			self.files.append(self.file)
			self.file = None
		raw = _Raw(None, self.files, self.written)
		self.files = []
		return raw

	def discard(self):
		self._drop()


def _close_raws(obj):
	"""Close encodings nested in a frame. Idempotent, and it does not walk a student's value."""
	if isinstance(obj, _Raw):
		obj.close()
	elif isinstance(obj, dict):
		for item in obj.values():
			_close_raws(item)
	elif isinstance(obj, (list, tuple)):
		for item in obj:
			_close_raws(item)


def write_all(fd, data):
	view = memoryview(data)
	while view:
		view = view[os.write(fd, view) :]


def write_frame(fd, obj):
	"""One length-prefixed frame, body written a chunk at a time."""
	cap = _Capture(None, False)
	raw = None
	try:
		_write_json(cap, obj)
		raw = cap.finish()
		write_all(fd, HEADER.pack(raw.size))
		raw.write_to(fd)
	finally:
		if raw is not None:
			raw.close()
		else:
			cap.discard()


def send(fd, obj):
	try:
		write_frame(fd, obj)
	finally:
		_close_raws(obj)


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
	value, extra, wired = None, None, None
	keep = False
	try:
		if _HAS_TIMER and timeout:
			signal.setitimer(signal.ITIMER_REAL, timeout, 0.05)
		try:
			value = fn()
			if sys.stdout is not out:
				# The student replaced stdout with their own wrapper: flush it in their scope.
				sys.stdout.flush()
			wired = wire(value) if serialise else None
			outcome = {"returned": {"value": wired, "type": type(value).__name__}}
			if then is not None:
				extra = then(value)
			keep = True
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
		keep = False
		outcome = {"timeout": {}}
	if not keep:
		# The frame that would have carried these is not sent.
		_close_raws(wired)
		_close_raws(extra)
		if isinstance(value, _Raw):
			value.close()
		extra = None
	elapsed = int((time.perf_counter() - start) * 1000)
	stdout, truncated = captured(out, raw)
	return Call(outcome, stdout, truncated, elapsed, value, extra if keep else None)


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


_ESCAPES = {
	0x22: b'\\"',
	0x5C: b"\\\\",
	0x08: b"\\b",
	0x0C: b"\\f",
	0x0A: b"\\n",
	0x0D: b"\\r",
	0x09: b"\\t",
}


def _write_string(out, text):
	"""A JSON string, escaped a slice at a time. Lone surrogates become U+FFFD, as the
	frame's `utf-8` with `replace` did."""
	out.write(b'"')
	n = len(text)
	i = 0
	step = 8192
	while i < n:
		end = min(i + step, n)
		raw = i
		while i < end:
			code = ord(text[i])
			if code < 0x20 or code in _ESCAPES:
				if i > raw:
					out.write(text[raw:i].encode("utf-8", "replace"))
				if code in _ESCAPES:
					out.write(_ESCAPES[code])
				else:
					out.write(f"\\u{code:04x}".encode("ascii"))
				i += 1
				raw = i
			else:
				i += 1
		if i > raw:
			out.write(text[raw:i].encode("utf-8", "replace"))
	out.write(b'"')


def _write_json(out, value):
	"""Compact JSON of a frame's own tree. A `_Raw` is spliced, not loaded."""
	if value is None:
		out.write(b"null")
	elif isinstance(value, bool):
		out.write(b"true" if value else b"false")
	elif isinstance(value, str):
		_write_string(out, value)
	elif isinstance(value, int):
		out.write(str(int(value)).encode("ascii"))
	elif isinstance(value, float):
		out.write(json.dumps(value, ensure_ascii=False, allow_nan=True).encode("ascii"))
	elif isinstance(value, _Raw):
		out.write_raw(value)
	elif isinstance(value, dict):
		out.write(b"{")
		first = True
		for key, item in value.items():
			if not first:
				out.write(b",")
				out.space()
			first = False
			_write_string(out, key)
			out.write(b":")
			out.space()
			_write_json(out, item)
		out.write(b"}")
	elif isinstance(value, (list, tuple)):
		out.write(b"[")
		first = True
		for item in value:
			if not first:
				out.write(b",")
				out.space()
			first = False
			_write_json(out, item)
		out.write(b"]")
	else:
		raise Unserialisable(f"cannot frame a {type(value).__name__}")


def _write_wire(out, value, depth, stack):
	"""The record encoding, in the bytes `json.dumps` of `to_json` would produce."""
	if value is None:
		out.write(b"null")
		return
	if isinstance(value, bool):
		out.write(b"true" if value else b"false")
		return
	if isinstance(value, str):
		_write_string(out, value)
		return
	if isinstance(value, int):
		if -(2**63) <= value < 2**64:
			out.write(str(int(value)).encode("ascii"))
		else:
			try:
				text = int.__repr__(value)
			except ValueError:
				text = hex(value)
			out.write(b'{"$bigint":')
			out.space()
			_write_string(out, text)
			out.write(b"}")
		return
	if isinstance(value, float):
		if math.isfinite(value):
			out.write(json.dumps(value, ensure_ascii=False, allow_nan=False).encode("ascii"))
		else:
			_write_string(out, repr(value))
		return
	if depth >= DEPTH_LIMIT:
		_write_string(out, "<too deep>")
		return
	identity = id(value)
	if identity in stack:
		_write_string(out, "<cycle>")
		return
	if isinstance(value, (list, tuple)):
		stack.add(identity)
		try:
			out.write(b"[")
			first = True
			for item in value:
				if not first:
					out.write(b",")
					out.space()
				first = False
				_write_wire(out, item, depth + 1, stack)
			out.write(b"]")
		finally:
			stack.discard(identity)
		return
	if isinstance(value, (set, frozenset)):
		# Sorting needs every element: stream the sorted tree, don't sort a second copy.
		stack.add(identity)
		try:
			items = [to_json(item, depth + 1, stack) for item in value]
			try:
				items.sort()
			except TypeError:
				items.sort(key=lambda item: json.dumps(item, sort_keys=True))
			_write_json(out, items)
		finally:
			stack.discard(identity)
		return
	if isinstance(value, dict):
		stack.add(identity)
		try:
			out.write(b"{")
			first = True
			seen = set()
			for key, item in value.items():
				name = key if isinstance(key, str) else text_of(key)
				if name in seen:
					raise Unserialisable(f"two keys both read as '{name}'")
				seen.add(name)
				if not first:
					out.write(b",")
					out.space()
				first = False
				_write_string(out, name)
				out.write(b":")
				out.space()
				_write_wire(out, item, depth + 1, stack)
			out.write(b"}")
		finally:
			stack.discard(identity)
		return
	_write_string(out, text_of(value))


def wire(value):
	"""The value as a record carries it. One too large to report is replaced by its size:
	the call still returned, and a function checker still judges it through the transfer
	encoding. The size is `json.dumps` without compact separators, counted while the compact
	bytes are written; past the limit the bytes are dropped and only the count continues."""
	cap = _Capture(VALUE_LIMIT, True)
	try:
		_write_wire(cap, value, 0, set())
	except BaseException:
		cap.discard()
		raise
	if cap.over:
		size = cap.written + cap.extra
		cap.discard()
		return {"$too_large": f"{size} bytes of JSON"}
	return cap.finish()


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


class _Walk:
	"""Ids assigned on the first visit. `keep` holds those objects so an id cannot be reused
	before the walk ends; `building` is the tuple or frozenset whose items are still open."""

	__slots__ = ("ids", "keep", "counter", "building")

	def __init__(self):
		self.ids = {}
		self.keep = []
		self.counter = 0
		self.building = set()

	def register(self, value):
		number = self.counter
		self.counter += 1
		self.ids[id(value)] = number
		self.keep.append(value)
		return number


def _write_ref(out, number):
	out.write(b'{"$":"ref","#":')
	out.write(str(number).encode("ascii"))
	out.write(b"}")


def _write_text(out, text, kind):
	out.write(b'{"$":"text","v":')
	_write_string(out, text)
	out.write(b',"type":')
	_write_string(out, kind)
	out.write(b"}")


def _write_int_value(out, value):
	if -(2**63) <= value < 2**64:
		out.write(str(int(value)).encode("ascii"))
	else:
		out.write(b'{"$":"int","v":')
		_write_string(out, hex(value))
		out.write(b"}")


def _write_b64(out, kind, value):
	out.write(b'{"$":"')
	out.write(kind.encode("ascii"))
	out.write(b'","v":"')
	step = 3 * 8192
	view = memoryview(value)
	for start in range(0, len(view), step):
		out.write(base64.b64encode(view[start : start + step]))
	out.write(b'"}')


def _write_atom(out, value, depth, walk):
	if isinstance(value, decimal.Decimal):
		out.write(b'{"$":"decimal","v":')
		_write_string(out, str(value))
		out.write(b"}")
		return
	if isinstance(value, fractions.Fraction):
		out.write(b'{"$":"fraction","v":[')
		_write_transfer(out, value.numerator, depth + 1, walk)
		out.write(b",")
		_write_transfer(out, value.denominator, depth + 1, walk)
		out.write(b"]}")
		return
	if isinstance(value, complex):
		out.write(b'{"$":"complex","v":[')
		_write_string(out, repr(value.real))
		out.write(b",")
		_write_string(out, repr(value.imag))
		out.write(b"]}")
		return
	if isinstance(value, datetime.datetime):
		out.write(b'{"$":"datetime","v":')
		_write_string(out, value.isoformat())
		out.write(b"}")
		return
	if isinstance(value, datetime.date):
		out.write(b'{"$":"date","v":')
		_write_string(out, value.isoformat())
		out.write(b"}")
		return
	if isinstance(value, datetime.time):
		out.write(b'{"$":"time","v":')
		_write_string(out, value.isoformat())
		out.write(b"}")
		return
	if isinstance(value, datetime.timedelta):
		out.write(b'{"$":"timedelta","v":[')
		_write_int_value(out, value.days)
		out.write(b",")
		_write_int_value(out, value.seconds)
		out.write(b",")
		_write_int_value(out, value.microseconds)
		out.write(b"]}")
		return
	_write_b64(out, "bytes" if isinstance(value, bytes) else "bytearray", value)


def _write_items(out, values, depth, walk):
	first = True
	for item in values:
		if not first:
			out.write(b",")
		first = False
		_write_transfer(out, item, depth + 1, walk)


def _write_tagged(out, kind, number):
	out.write(b'{"$":"')
	out.write(kind.encode("ascii"))
	out.write(b'","#":')
	out.write(str(number).encode("ascii"))


def _write_str_map(out, mapping, depth, walk):
	out.write(b"{")
	first = True
	for key, item in mapping.items():
		if not first:
			out.write(b",")
		first = False
		_write_string(out, key)
		out.write(b":")
		_write_transfer(out, item, depth + 1, walk)
	out.write(b"}")


def _write_transfer(out, value, depth, walk):
	"""One pass: the first visit writes the value and records its id; a later visit writes
	the id. A tuple or frozenset is numbered for `decode` only after its items, so a way
	back into one while those items are written is `<cycle>`, as the record encoding says."""
	if value is None:
		out.write(b"null")
		return
	if isinstance(value, bool):
		out.write(b"true" if value else b"false")
		return
	if isinstance(value, str):
		_write_string(out, value)
		return
	if isinstance(value, int):
		_write_int_value(out, value)
		return
	if isinstance(value, float):
		if math.isfinite(value):
			out.write(json.dumps(value, ensure_ascii=False, allow_nan=False).encode("ascii"))
		else:
			out.write(b'{"$":"float","v":')
			_write_string(out, repr(value))
			out.write(b"}")
		return
	if isinstance(value, _ATOMS):
		_write_atom(out, value, depth, walk)
		return
	# Before the id: a visit past the depth limit is text even when the object was seen
	# nearer the root, and a visit that is only too deep does not take an id.
	if depth >= DEPTH_LIMIT:
		_write_text(out, "<too deep>", type(value).__name__)
		return
	identity = id(value)
	number = walk.ids.get(identity)
	if number is not None:
		# A tuple or frozenset is registered for a later sibling, but `decode` only records
		# its id once its items exist, so a reference from inside those items cannot be one.
		if identity in walk.building and isinstance(value, (tuple, frozenset)):
			kind = "tuple" if isinstance(value, tuple) else "frozenset"
			_write_text(out, "<cycle>", kind)
		else:
			_write_ref(out, number)
		return
	shared = _shared_name(type(value))
	if shared is not None:
		number = walk.register(value)
		state = _state(value)
		_write_tagged(out, "shared", number)
		out.write(b',"class":')
		_write_string(out, shared)
		out.write(b',"state":')
		_write_str_map(out, state, depth, walk)
		out.write(b"}")
		return
	if isinstance(value, (tuple, frozenset)):
		kind = "tuple" if isinstance(value, tuple) else "frozenset"
		number = walk.register(value)
		walk.building.add(identity)
		try:
			_write_tagged(out, kind, number)
			out.write(b',"v":[')
			_write_items(out, value, depth, walk)
			out.write(b"]}")
		finally:
			walk.building.discard(identity)
		return
	if isinstance(value, (list, set)):
		kind = "list" if isinstance(value, list) else "set"
		number = walk.register(value)
		_write_tagged(out, kind, number)
		out.write(b',"v":[')
		_write_items(out, value, depth, walk)
		out.write(b"]}")
		return
	if isinstance(value, dict):
		number = walk.register(value)
		_write_tagged(out, "dict", number)
		out.write(b',"v":[')
		first = True
		for key, item in value.items():
			if not first:
				out.write(b",")
			first = False
			out.write(b"[")
			_write_transfer(out, key, depth + 1, walk)
			out.write(b",")
			_write_transfer(out, item, depth + 1, walk)
			out.write(b"]")
		out.write(b"]}")
		return
	if inspect.isroutine(value) or inspect.isclass(value) or inspect.ismodule(value):
		_write_text(out, text_of(value), type(value).__name__)
		return
	module = getattr(type(value), "__module__", "") or ""
	if module.split(".")[0] == "numpy" and callable(getattr(value, "tolist", None)):
		# Not registered: two references become two lists, as they did before.
		_write_transfer(out, value.tolist(), depth, walk)
		return
	if hasattr(value, "__dict__") or any("__slots__" in cls.__dict__ for cls in type(value).__mro__[:-1]):
		number = walk.register(value)
		attrs = _state(value)
		props = _properties(value)
		_write_tagged(out, "object", number)
		out.write(b',"class":')
		_write_string(out, type(value).__qualname__)
		out.write(b',"attrs":')
		_write_str_map(out, attrs, depth, walk)
		out.write(b',"properties":')
		_write_str_map(out, props, depth, walk)
		out.write(b"}")
		return
	_write_text(out, text_of(value), type(value).__name__)


def _capture_transfer(value, limit):
	cap = _Capture(limit, False)
	try:
		_write_transfer(cap, value, 0, _Walk())
	except BaseException:
		cap.discard()
		raise
	return cap


def encode(value):
	"""The transfer encoding of `value`, as JSON bytes. Runs the owner's code (properties,
	`__iter__` of a container subclass) and raises what it raises; call it under the owner's
	timer. The first visit of a container writes it and records its id."""
	return _capture_transfer(value, None).finish()


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
	"""The transfer encoding of `value`, or a marker saying how large it would have been.
	Past the limit the bytes already written are dropped; the walk keeps counting."""
	cap = _capture_transfer(value, TRANSFER_LIMIT)
	if cap.over:
		size = cap.written
		cap.discard()
		return {"$": "too_large", "size": size}
	return cap.finish()
