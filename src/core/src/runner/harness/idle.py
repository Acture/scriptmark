# The student ScriptMark measures a unit's idle memory with: it imports the modules the
# submission imports — read from its source beforehand, in another process — and waits
# while the grader samples it. `memory_mb` counts beyond what this holds, so no import is
# the student's. The grader samples rather than asking: on Linux `ru_maxrss` also counts
# the grader itself, whose memory the process held between fork and exec.
import time


def idle(imports):
	for name, names in imports:
		try:
			__import__(name, fromlist=names)
		except Exception:  # refused, missing or broken: it costs the student nothing either
			pass
	time.sleep(0.05)
