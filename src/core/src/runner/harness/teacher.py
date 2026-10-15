"""The teacher side of a unit: teacher modules and checkers, the plan, and the records.

It drives the student side one request at a time, sets a deadline on each, and reports
observations, never verdicts about who is at fault: the grader derives those from which
call was running, what kind of code it runs, and how each side ended. What the student
side answers is the student's evidence; it is checked against the largest answer that
request allows before it is read, and it never reaches a record unchecked.
"""

import importlib.util  # noqa: E402
import select  # noqa: E402

with open(sys.argv[1], encoding="utf-8") as _fh:
	PAYLOAD = json.load(_fh)
os.unlink(sys.argv[1])
configure(PAYLOAD["limits"])

_RECORDS = PAYLOAD["records"]
# Seconds a request may run past its own timeout before the student side is given up on,
# and how long the student side has to start.
GRACE = PAYLOAD["grace"]
STARTUP = PAYLOAD["startup"]
# The most a frame from the student side carries besides its value, stdout and files:
# the call's target and matching evidence, and the keys around them.
SLACK = 1 << 20


def emit(record):
	write_all(_RECORDS, frame(record))


def finish(student=None):
	"""Stop now: threads teacher code left running must not hold the unit open."""
	if student is not None:
		student.close()
	emit({"kind": "done"})
	os._exit(0)


# --- The student side -----------------------------------------------------------------
class Stop(Exception):
	"""The student side cannot be asked anything more: it went away, ran past a deadline,
	or answered with something no request allows."""

	def __init__(self, reason, message):
		super().__init__(message)
		self.reason = reason
		self.message = message


class Student:
	"""The pipe to the student side. One request is pending at a time: its answer is read
	before its deadline, and its length checked against the request's bound before its body
	is read. Reading needs no thread: the kernel's process limit counts threads, and a
	busy machine may have none to spare."""

	def __init__(self, read_fd, write_fd):
		self.read_fd, self.write_fd = read_fd, write_fd
		self.stopped = None
		self.windows = os.name == "nt"
		if self.windows:
			# Pipes cannot be polled there: a reader thread waits instead, and the process
			# limit that makes threads scarce is a unix one.
			import queue
			import threading

			self.answers = queue.Queue()
			threading.Thread(target=self._pump, daemon=True).start()

	def _pump(self):
		try:
			while True:
				(size,) = HEADER.unpack(read_exact(self.read_fd, HEADER.size))
				self.answers.put(size)
				self.answers.put(read_exact(self.read_fd, size) if size <= self.bound else None)
				if size > self.bound:
					return
		except (Closed, OSError):
			self.answers.put(Closed())

	def _exactly(self, n, deadline):
		"""`n` bytes from the student side, before `deadline`."""
		chunks, got = [], 0
		while got < n:
			left = deadline - time.monotonic()
			if left <= 0:
				raise TimeoutError()
			if not select.select([self.read_fd], [], [], left)[0]:
				continue
			chunk = os.read(self.read_fd, min(n - got, 1 << 20))
			if not chunk:
				raise Closed()
			chunks.append(chunk)
			got += len(chunk)
		return b"".join(chunks)

	def _answer(self, bound, timeout):
		deadline = time.monotonic() + timeout
		if self.windows:
			import queue

			try:
				size = self.answers.get(timeout=timeout)
				body = None if isinstance(size, Closed) or size > bound else self.answers.get(timeout=max(deadline - time.monotonic(), 0.001))
			except queue.Empty:
				raise TimeoutError() from None
			if isinstance(size, Closed) or isinstance(body, Closed):
				raise Closed()
		else:
			(size,) = HEADER.unpack(self._exactly(HEADER.size, deadline))
			body = None if size > bound else self._exactly(size, deadline)
		if body is None:
			raise Stop("flooded", f"the student side sent {size} bytes where at most {bound} may arrive")
		return body

	def ask(self, request, bound, timeout):
		if self.stopped is not None:
			raise self.stopped
		self.bound = bound
		try:
			send(self.write_fd, request)
		except OSError:
			self.stopped = Stop("exited", "the student side ended")
			raise self.stopped from None
		try:
			answer = self._answer(bound, timeout)
		except TimeoutError:
			self.stopped = Stop("deadline", f"the student side did not answer within {timeout:g}s")
			raise self.stopped from None
		except (Closed, OSError):
			self.stopped = Stop("exited", "the student side ended")
			raise self.stopped from None
		except Stop as stop:
			self.stopped = stop
			raise
		try:
			parsed = parse(answer)
			if not isinstance(parsed, dict):
				raise Garbled("an answer that is not an object")
			return parsed
		except Garbled as exc:
			self.stopped = Stop("garbled", str(exc))
			raise self.stopped from None

	def garbled(self, message):
		self.stopped = Stop("garbled", message)
		return self.stopped

	def close(self):
		if self.stopped is None:
			try:
				send(self.write_fd, {"op": "exit"})
			except OSError:
				pass


def call_bound(files=0, value=True, transfer=False):
	"""The largest answer a call can legitimately make: its value, its stdout and an error
	message, each of which JSON escaping can grow sixfold, and each file it observes."""
	bound = SLACK + 6 * STDOUT_LIMIT + 6 * MESSAGE_LIMIT + files * (6 * FILE_LIMIT + 4096)
	if value:
		bound += VALUE_LIMIT
	if transfer:
		bound += TRANSFER_LIMIT
	return bound


# --- Checking what the student side says -------------------------------------------------
# The outcomes each kind of request can have. The student side resolves nothing by name — this
# side does, before asking — and only a fuzzy function lookup can need a teacher's decision:
# anything else claimed is not an answer to the request.
RAN = {"returned", "raised", "timeout"}
LOOKED_UP = RAN | {"missing", "unserialisable"}
MATCHED = LOOKED_UP | {"matching"}


def _str(value, what):
	if not isinstance(value, str):
		raise Garbled(f"{what} is not text")
	return value


def _error(info, what):
	if not isinstance(info, dict):
		raise Garbled(f"{what} is not an error")
	error = {
		"type": _str(info.get("type"), what),
		"types": [_str(t, what) for t in info.get("types", [])] if isinstance(info.get("types", []), list) else None,
		"message": _str(info.get("message", ""), what),
	}
	if error["types"] is None:
		raise Garbled(f"{what} has no list of types")
	if "clean" in info:
		if not isinstance(info["clean"], bool):
			raise Garbled(f"{what} says neither clean nor not")
		error["clean"] = info["clean"]
	return error


def _decision(decision):
	if decision is None:
		return None
	if not isinstance(decision, dict) or not isinstance(decision.get("candidates"), list):
		raise Garbled("matching evidence that is not a decision")
	selected = decision.get("selected")
	if selected is not None:
		_str(selected, "a selected function")
	return {
		"state": _str(decision.get("state"), "a matching state"),
		"selected": selected,
		"candidates": [
			{
				"value": _str(c.get("value"), "a candidate"),
				"rules": [_str(r, "a rule") for r in c.get("rules", [])],
			}
			for c in decision["candidates"]
			if isinstance(c, dict) and isinstance(c.get("rules", []), list)
		],
	}


def _outcome(outcome, allowed):
	if not isinstance(outcome, dict) or len(outcome) != 1:
		raise Garbled("an outcome that is not exactly one thing")
	((kind, body),) = outcome.items()
	if kind not in allowed or not isinstance(body, dict):
		raise Garbled(f"an outcome '{kind}' this request cannot have")
	if kind == "returned":
		return {"returned": {"value": body.get("value"), "type": _str(body.get("type"), "a type name")}}
	if kind in ("raised", "unserialisable"):
		return {kind: _error(body, "an error")}
	if kind == "timeout":
		return {"timeout": {}}
	if kind == "missing":
		return {"missing": {"message": _str(body.get("message"), "a message")}}
	decision = _decision(body.get("decision"))
	if decision is None or decision["selected"] is not None:
		raise Garbled("a lookup that needs a decision but settled on a function")
	return {"matching": {"message": _str(body.get("message"), "a message"), "decision": decision}}


def checked(answer, files, allowed):
	"""The call record an answer describes, rebuilt from only what a record may hold."""
	if "fatal" in answer:
		raise Garbled(f"the student side failed: {text_of(answer['fatal'])[:MESSAGE_LIMIT]}")
	stdout = _str(answer.get("stdout", ""), "stdout")
	truncated = answer.get("stdout_truncated", False)
	elapsed = answer.get("elapsed_ms", 0)
	if not isinstance(truncated, bool) or not isinstance(elapsed, int) or isinstance(elapsed, bool):
		raise Garbled("a call record with malformed fields")
	record = {
		"outcome": _outcome(answer.get("outcome"), allowed),
		"stdout": stdout,
		"stdout_truncated": truncated,
		"elapsed_ms": max(elapsed, 0),
	}
	target = answer.get("target")
	if target is not None:
		if not isinstance(target, dict):
			raise Garbled("a target that is not an object")
		record["target"] = {
			"requested": _str(target.get("requested"), "a target"),
			"resolved": _str(target.get("resolved"), "a target"),
			"matching": _decision(target.get("matching")),
		}
	if files:
		seen = answer.get("files")
		if not isinstance(seen, dict) or set(seen) != set(files):
			raise Garbled("observed files other than the ones asked for")
		record["files"] = {
			path: None if content is None else _str(content, "a file's content")
			for path, content in seen.items()
		}
	return record


# --- Teacher modules -------------------------------------------------------------------
def _removed_checker(*args, **kwargs):
	raise RuntimeError(
		"@checker no longer binds a checker to a function: put "
		'check = { function = "<checker name>" } on each case it should judge '
		"(https://github.com/Acture/obsidian-vault/blob/project/scriptmark/"
		"docs/test-bundles.md#migrating-an-older-spec)"
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


def load_shared():
	"""The teacher's own copy of each shared module, importable under its name, so teacher
	modules that import it share its classes with what the student side sends."""
	for path in PAYLOAD.get("shared", []):
		name = os.path.splitext(os.path.basename(path))[0]
		spec = importlib.util.spec_from_file_location(name, path)
		module = importlib.util.module_from_spec(spec)
		sys.modules[name] = module
		spec.loader.exec_module(module)
		share(module)


def load_teacher_modules():
	"""Returns (exports, duplicates): a name two modules export differently is refused."""
	exports, owner, duplicates = {}, {}, {}
	builtins.checker = _removed_checker
	try:
		load_shared()
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


# --- Values ------------------------------------------------------------------------------
# Values held on this side: teacher exports, `[vars]`, and what teacher calls made, by name.
context = {}
# Names whose values live on the student side: what the student's calls made.
held = set()
SOURCE = None


def bind(name, value=None, student=False):
	if student:
		context.pop(name, None)
		held.add(name)
	else:
		held.discard(name)
		context[name] = value


def outgoing(value):
	"""A plan argument as the student side receives it: a `$name` the student side holds
	is named, one this side holds is copied, and `$$` escapes a literal `$`."""
	if isinstance(value, str) and value.startswith("$"):
		if value.startswith("$$"):
			return value[1:]
		name = value[1:]
		if name in held:
			return {"$": "id", "name": name}
		if name in context:
			return encode(context[name])
		raise Unresolved(name)
	if isinstance(value, list):
		return [outgoing(v) for v in value]
	if isinstance(value, dict):
		pairs = [(k, outgoing(v)) for k, v in value.items()]
		if "$" in value:
			return {"$": "dict", "v": [[k, v] for k, v in pairs]}
		return dict(pairs)
	return value


def fetch(student, name, timeout):
	"""A value the student side holds, taken by value now. Returns the value, or the
	outcome that kept it from arriving."""
	answer = student.ask({"op": "fetch", "name": name, "timeout": timeout}, call_bound(value=False, transfer=True), timeout + GRACE)
	try:
		if "value" in answer:
			tree = answer["value"]
			if isinstance(tree, dict) and tree.get("$") == "too_large":
				return None, {"error": problem("TooLarge", f"'{name}' is {tree.get('size')} bytes of JSON, more than a checker can be given")}
			return decode(tree), None
		outcome = _outcome(answer.get("outcome"), RAN)
	except Garbled as exc:
		raise student.garbled(str(exc)) from None
	if "timeout" in outcome:
		return None, {"error": problem("Timeout", f"reading '{name}' for the checker took longer than {timeout}s")}
	if "raised" in outcome:
		return None, {"error": outcome["raised"]}
	return None, {"error": problem("Unreadable", f"'{name}' could not be read for the checker")}


def local(value, student, timeout):
	"""A plan argument for teacher code: student-side values are fetched by value."""
	if isinstance(value, str) and value.startswith("$"):
		if value.startswith("$$"):
			return value[1:]
		name = value[1:]
		if name in context:
			return context[name]
		if name in held:
			got, failed = fetch(student, name, timeout)
			if failed is not None:
				raise Unresolved(name)
			return got
		raise Unresolved(name)
	if isinstance(value, list):
		return [local(v, student, timeout) for v in value]
	if isinstance(value, dict):
		return {k: local(v, student, timeout) for k, v in value.items()}
	return value


# --- Checks ------------------------------------------------------------------------------
def verdict_of(returned):
	if isinstance(returned, bool):
		return {"verdict": {"pass": returned, "message": ""}}
	if (
		isinstance(returned, tuple)
		and len(returned) == 2
		and isinstance(returned[0], bool)
		and isinstance(returned[1], (str, type(None)))
	):
		return {"verdict": {"pass": returned[0], "message": clip(returned[1] or "")}}
	return {
		"error": {
			"type": "CheckerContract",
			"types": ["CheckerContract"],
			"message": f"checker returned {type(returned).__name__}; expected bool or (bool, str)",
		}
	}


def run_check(check, value, stdout, timeout, student):
	fn = context.get(check["function"])
	deps = {}
	for param in (params_of(fn) or [])[2:]:
		name = param["name"]
		if "var" in param["kind"]:
			continue
		if name == "stdout":
			deps[name] = stdout
		elif name == "source" and SOURCE is not None and name not in context and name not in held:
			deps[name] = SOURCE
		elif name in context:
			deps[name] = context[name]
		elif name in held:
			got, failed = fetch(student, name, timeout)
			if failed is not None:
				return failed
			deps[name] = got
		elif not param["default"]:
			# prepare saw this name in scope: it is an id whose producing call failed.
			return {"unresolved": {"name": name}}
	done = call(lambda: fn(value, check.get("expected"), **deps), timeout, serialise=False)
	if "returned" in done.outcome:
		return verdict_of(done.value)
	if "timeout" in done.outcome:
		return {"error": problem("Timeout", f"checker timed out after {timeout}s")}
	raised = done.outcome.get("raised") or {"type": "Error", "types": [], "message": str(done.outcome)}
	if "AssertionError" in raised["types"]:
		return {"rejected": {"message": raised["message"]}}
	return {"error": raised}


def judged_value(answer, record, student):
	"""The value a checker judges: rebuilt from the transfer encoding when the student side
	sent one, otherwise from the record, which then says the same thing."""
	tree = answer["transfer"] if "transfer" in answer else record["outcome"]["returned"]["value"]
	if isinstance(tree, dict) and tree.get("$") == "too_large":
		return None, {"error": problem("TooLarge", f"the value is {tree.get('size')} bytes of JSON, more than a checker can be given")}
	try:
		return decode(tree), None
	except Garbled as exc:
		raise student.garbled(str(exc)) from None


# --- Calls -------------------------------------------------------------------------------
def stopped(phase, index, stop):
	emit({"kind": "stopped", "phase": phase, "index": index, "reason": stop.reason, "message": clip(stop.message)})


def teacher_call(plan, phase, index, student):
	name = plan["target"]["teacher"]["name"]
	fn = context.get(name)
	try:
		# Student-side arguments are read before the teacher's call and its timer starts.
		args = local(plan.get("args", []), student, plan["timeout"])
	except Unresolved as exc:
		args = exc
	except Garbled as exc:
		stopped(phase, index, student.garbled(str(exc)))
		return None
	except Stop as stop:
		stopped(phase, index, stop)
		return None

	def run():
		if isinstance(args, Unresolved):
			raise args
		if fn is None or not callable(fn):
			raise Missing(f"teacher function '{name}' not found")
		return fn(*args)

	done = call(run, plan["timeout"], plan.get("stdin"), serialise=phase != "setup")
	record = {"kind": "call", "phase": phase, "index": index, **done.record()}
	record["target"] = {"requested": name, "resolved": name, "matching": None}
	emit(record)
	if "returned" in done.outcome and plan.get("id"):
		bind(plan["id"], done.value)
	return "returned" in done.outcome


def student_call(plan, phase, index, student):
	kind, spec = next(iter(plan["target"].items()))
	files = plan.get("files") or []
	check = plan.get("check") if phase == "step" else None
	request = {
		"op": "call",
		"target": plan["target"],
		"timeout": plan["timeout"],
		"stdin": plan.get("stdin"),
		"id": plan.get("id"),
		"files": files,
		"serialise": phase != "setup",
		"transfer": check is not None,
	}
	try:
		request["args"] = outgoing(plan.get("args", []))
		if kind in ("method", "attribute") and spec["object"] not in held:
			if spec["object"] not in context:
				raise Unresolved(spec["object"])
			request["object"] = encode(context[spec["object"]])
	except Unresolved as exc:
		emit({
			"kind": "call", "phase": phase, "index": index,
			"target": {"requested": spec["name"], "resolved": spec["name"], "matching": None},
			"outcome": {"unresolved": {"name": str(exc)}},
			"stdout": "", "stdout_truncated": False, "elapsed_ms": 0,
		})
		return False
	if kind != "function":
		allowed = LOOKED_UP
	elif PAYLOAD["role"] == "student":
		allowed = MATCHED
	else:
		allowed = LOOKED_UP
	try:
		answer = student.ask(request, call_bound(len(files), transfer=check is not None), plan["timeout"] + GRACE)
		try:
			record = checked(answer, files, allowed)
		except Garbled as exc:
			raise student.garbled(str(exc)) from None
		returned = "returned" in record["outcome"]
		value, failed = (None, None)
		if returned and check is not None:
			value, failed = judged_value(answer, record, student)
	except Stop as stop:
		if stop.reason == "deadline":
			# The call's own timer should have ended it: the call ran past its deadline.
			emit({
				"kind": "call", "phase": phase, "index": index,
				"target": {"requested": spec["name"], "resolved": spec["name"], "matching": None},
				"outcome": {"timeout": {}}, "stdout": "", "stdout_truncated": False,
				"elapsed_ms": int((plan["timeout"] + GRACE) * 1000),
			})
		stopped(phase, index, stop)
		return None
	emit({"kind": "call", "phase": phase, "index": index, **record})
	if returned and plan.get("id"):
		bind(plan["id"], student=True)
	if returned and check is not None:
		try:
			try:
				verdict = failed or run_check(check, value, record["stdout"], plan["timeout"], student)
			except Garbled as exc:
				raise student.garbled(str(exc)) from None
		except Stop as stop:
			emit({"kind": "check", "index": index, "error": problem("StudentEnded", f"the student side stopped while the checker read its values: {stop.message}")})
			stopped(phase, index, stop)
			return None
		emit({"kind": "check", "index": index, **verdict})
	return returned


def run_calls(phase, calls, student):
	"""Run setup calls or steps in order. Returns False when a setup call failed, and None
	when the student side was stopped."""
	for index, plan in enumerate(calls):
		if "teacher" in plan["target"]:
			returned = teacher_call(plan, phase, index, student)
		else:
			returned = student_call(plan, phase, index, student)
		if returned is None:
			return None
		if phase == "setup" and not returned:
			return False
	return True


def run_load(student):
	try:
		answer = student.ask({"op": "load", "timeout": PAYLOAD["load_timeout"]}, call_bound(value=False), PAYLOAD["load_timeout"] + GRACE)
		record = checked(answer, [], RAN)
	except Garbled as exc:
		stopped("load", 0, student.garbled(str(exc)))
		return False
	except Stop as stop:
		if stop.reason == "deadline":
			emit({"kind": "call", "phase": "load", "index": 0, "outcome": {"timeout": {}},
				"stdout": "", "stdout_truncated": False, "elapsed_ms": int((PAYLOAD["load_timeout"] + GRACE) * 1000)})
		stopped("load", 0, stop)
		return False
	emit({"kind": "call", "phase": "load", "index": 0, **record})
	return "returned" in record["outcome"]


def run_script(student):
	script = PAYLOAD["script"]
	files = script.get("files") or []
	try:
		answer = student.ask(
			{"op": "script", "timeout": script["timeout"], "stdin": script.get("stdin"), "files": files},
			call_bound(len(files), value=False),
			script["timeout"] + GRACE,
		)
		compile_error = answer.get("compiles") is False
		record = checked(answer, [] if compile_error else files, RAN)
	except Garbled as exc:
		stopped("step", 0, student.garbled(str(exc)))
		return
	except Stop as stop:
		if stop.reason == "deadline":
			emit({"kind": "call", "phase": "step", "index": 0, "outcome": {"timeout": {}},
				"stdout": "", "stdout_truncated": False, "elapsed_ms": int((script["timeout"] + GRACE) * 1000)})
		stopped("step", 0, stop)
		return
	# A script that does not compile never ran: that is its load failing.
	emit({"kind": "call", "phase": "load" if compile_error else "step", "index": 0, **record})


def start_student():
	"""Introduce the unit to the student side and wait for it to say it is ready."""
	student = Student(*channel(0, 1))
	init = {
		"limits": PAYLOAD["limits"],
		"role": PAYLOAD["role"],
		"subject": PAYLOAD["subject"],
		"vars": PAYLOAD.get("vars", {}),
		"allowed_imports": PAYLOAD.get("allowed_imports", [])
		+ [os.path.splitext(os.path.basename(p))[0] for p in PAYLOAD.get("student_shared", [])],
		"functions": PAYLOAD.get("functions", {"aliases": {}, "overrides": {}}),
		"shared": PAYLOAD.get("student_shared", []),
		"canary": PAYLOAD.get("student_canary"),
	}
	try:
		answer = student.ask(init, SLACK + 6 * MESSAGE_LIMIT, STARTUP)
	except Stop as stop:
		emit({"kind": "fatal", "stage": "student", "error": problem("StudentStart", f"the student side did not start: {stop.message}")})
		finish()
	if answer.get("ready") is not True:
		fatal = answer.get("fatal")
		error = fatal if isinstance(fatal, dict) else {}
		stage = "sandbox" if error.get("type") == "Sandbox" else "student"
		emit({"kind": "fatal", "stage": stage, "error": problem(str(error.get("type", "StudentStart")), str(error.get("message", "the student side did not start")))})
		finish()
	return student


def canary_holds():
	path = PAYLOAD.get("canary")
	if not path:
		return True
	try:
		with open(path, "rb"):
			return False
	except OSError:
		return True


def main():
	global SOURCE
	if not canary_holds():
		emit({"kind": "fatal", "stage": "sandbox", "error": problem("Sandbox", "the teacher sandbox did not hold: its canary was readable")})
		finish()
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
	if PAYLOAD.get("source"):
		with open(PAYLOAD["source"], encoding="utf-8", errors="replace") as fh:
			SOURCE = fh.read()
	student = start_student()
	emit({"kind": "ready"})
	if PAYLOAD.get("script"):
		run_script(student)
		finish(student)
	if run_load(student):
		setup = run_calls("setup", PAYLOAD.get("setup", []), student)
		if setup:
			run_calls("step", PAYLOAD.get("steps", []), student)
	finish(student)


try:
	main()
except BaseException as exc:  # a harness bug, not anybody's code under test
	emit({"kind": "fatal", "stage": "harness", "error": error_of(exc)})
	finish()
