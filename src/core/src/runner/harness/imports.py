# Reads a submission's source on stdin and prints, as JSON, each module an import in it
# names, wherever in the file, with what `from … import` takes from it. Nothing in the
# source runs. It runs in a process of its own, never the one whose memory is measured:
# what parsing takes is the source's to decide.
import ast
import json
import sys


def imports(text):
	found = []
	for node in ast.walk(ast.parse(text)):
		if isinstance(node, ast.Import):
			found.extend([alias.name, []] for alias in node.names)
		elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
			found.append([node.module, [a.name for a in node.names if a.name != "*"]])
	return found


try:
	found = imports(sys.stdin.buffer.read().decode("utf-8", "replace"))
except (SyntaxError, ValueError, RecursionError, MemoryError):  # it imports nothing beforehand
	found = []
print(json.dumps(found))
