# Reads a submission's source on stdin and prints, as JSON, each module an import in it
# names, wherever in the file, with what `from … import` takes from it. Nothing in the
# source runs. It reads tokens as they come and keeps only those of import statements, so
# what it holds does not grow with the source, whatever the source holds.
import json
import sys
import tokenize

_STARTS = {tokenize.NEWLINE, tokenize.INDENT, tokenize.DEDENT}
_SKIPPED = {tokenize.NL, tokenize.COMMENT, tokenize.ENCODING}


def _module(words):
	"""`import a.b as c, d` or `from a.b import (x as y, z)`: the module, and what it takes."""
	if words[0] == "import":
		modules, name, renamed = [], [], False
		for word in words[1:] + [","]:
			if word == ",":
				if name:
					modules.append(["".join(name), []])
				name, renamed = [], False
			elif word == "as":
				renamed = True
			elif not renamed:
				name.append(word)
		return modules
	if "import" not in words or words[1] in (".", "..."):  # relative: the submission's own
		return []
	at = words.index("import")
	names, renamed = [], False
	for word in words[at + 1 :]:
		if word in ("(", ")", "*"):
			continue
		if word == ",":
			renamed = False
		elif word == "as":
			renamed = True
		elif not renamed:
			names.append(word)
	return [["".join(words[1:at]), names]]


def imports(readline):
	found = []
	start, nesting, statement = True, 0, None
	try:
		for token in tokenize.tokenize(readline):
			kind, text = token.type, token.string
			if kind in _SKIPPED:
				continue
			ends = kind in _STARTS or kind == tokenize.ENDMARKER or (kind == tokenize.OP and text == ";" and not nesting)
			if ends:
				if statement:
					found.extend(_module(statement))
				start, statement = True, None
				continue
			if kind == tokenize.OP:
				nesting += text in "([{"
				nesting -= text in ")]}"
			if statement is not None:
				statement.append(text)
			elif start and kind == tokenize.NAME and text in ("import", "from"):
				statement = [text]
			# A statement after a colon on the same line: `if x: import y`.
			start = kind == tokenize.OP and text == ":" and not nesting
	except (tokenize.TokenError, SyntaxError, ValueError):  # what was read so far stands
		pass
	return found


print(json.dumps(imports(sys.stdin.buffer.readline)))
