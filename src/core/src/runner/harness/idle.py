# The student ScriptMark measures a unit's idle memory with: it imports what the submission
# imports, read from its source without running any of it, and waits while the grader
# samples it. `memory_mb` counts beyond what this holds, so no import is the student's.
# The grader samples rather than asking: on Linux `ru_maxrss` also counts the grader
# itself, whose memory the process held between fork and exec.
import ast
import time

SOURCE = ".scriptmark-source"


def _imports(path):
	try:
		with open(path, encoding="utf-8", errors="replace") as fh:
			tree = ast.parse(fh.read())
	except Exception:  # a submission that does not parse imports nothing here
		return []
	found = []
	for node in ast.walk(tree):
		if isinstance(node, ast.Import):
			found.extend((alias.name, ()) for alias in node.names)
		elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
			found.append((node.module, tuple(a.name for a in node.names if a.name != "*")))
	return found


for _name, _from in _imports(SOURCE):
	try:
		__import__(_name, fromlist=_from)
	except Exception:  # refused, missing or broken: it costs the student nothing either
		pass


def idle():
	time.sleep(0.05)
