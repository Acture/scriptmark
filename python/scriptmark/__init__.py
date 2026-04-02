"""ScriptMark — automated grading for student programming assignments."""

from scriptmark._scriptmark import (
	discover,
	grade,
	load_spec,
	run,
	StudentResult,
	TestSpec,
)

__all__ = [
	"discover",
	"grade",
	"load_spec",
	"run",
	"StudentResult",
	"TestSpec",
]
