"""The same function decisions for static preview and runtime lookup."""

import ast
import difflib
import json
import sys
import tokenize
from typing import TypedDict


class MatchCandidate(TypedDict):
	value: str
	rules: list[str]


class MatchDecision(TypedDict):
	state: str
	selected: str | None
	candidates: list[MatchCandidate]


class FunctionRules(TypedDict):
	aliases: dict[str, list[str]]
	overrides: dict[str, str]


def function_decision(
	names: list[str], requested: str, policy: FunctionRules
) -> MatchDecision:
	override: str | None = policy.get("overrides", {}).get(requested)
	if override is not None:
		found: list[str] = [override] if override in names else []
		rule: str = f"matching.overrides.functions.{requested} = {override!r}"
	elif requested in names:
		found = [requested]
		rule = "source.function_exact"
	else:
		found = sorted(
			set(names).intersection(policy.get("aliases", {}).get(requested, []))
		)
		rule = f"matching.items.functions.{requested} = {policy.get('aliases', {}).get(requested, [])!r}"
		if not found and requested not in policy.get("aliases", {}):
			found = sorted(
				name
				for name in set(names)
				if not name.startswith("_")
				and difflib.SequenceMatcher(
					None, requested.lower(), name.lower()
				).ratio()
				>= 0.5
			)
			rule = "suggestion.function_similarity"
	state: str = (
		"missing"
		if not found
		else (
			"review"
			if rule.startswith("suggestion.")
			else "matched"
			if len(found) == 1
			else "ambiguous"
		)
	)
	return {
		"state": state,
		"selected": found[0] if state == "matched" else None,
		"candidates": [{"value": name, "rules": [rule]} for name in found],
	}


def static_names(nodes: list[ast.stmt]) -> list[str]:
	"""Declared module functions/classes, including conditional declarations, never methods."""
	names: list[str] = []
	for node in nodes:
		if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
			names.append(node.name)
		else:
			for child in ast.iter_child_nodes(node):
				if isinstance(child, ast.stmt):
					names.extend(static_names([child]))
	return names


class PreviewRequest(TypedDict):
	path: str
	requested: list[str]
	policy: FunctionRules


def preview_functions() -> None:
	requests: list[PreviewRequest] = json.load(sys.stdin)
	results: list[dict[str, MatchDecision]] = []
	for request in requests:
		try:
			with tokenize.open(request["path"]) as source:
				names: list[str] = static_names(
					ast.parse(source.read(), filename=request["path"]).body
				)
		except (OSError, SyntaxError, UnicodeError, LookupError):
			# Syntax/loading faults belong to execution, not a teacher matching mistake.
			results.append(
				{
					name: {"state": "deferred", "selected": None, "candidates": []}
					for name in request["requested"]
				}
			)
			continue
		decisions: dict[str, MatchDecision] = {}
		for name in request["requested"]:
			decision: MatchDecision = function_decision(names, name, request["policy"])
			if decision["state"] == "missing":
				decision["state"] = (
					"deferred"  # imports, assignments, __getattr__ and decorators
				)
			decisions[name] = decision
		results.append(decisions)
	json.dump(results, sys.stdout, ensure_ascii=False, sort_keys=True)
