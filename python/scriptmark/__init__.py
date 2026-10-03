"""ScriptMark — automated grading for student programming assignments."""

from scriptmark._scriptmark import (
	discover,
	grade,
	load_input,
	load_record,
	load_spec,
	rescore,
	run,
	StudentResult,
	TestSpec,
)

__all__ = [
	"discover",
	"grade",
	"load_input",
	"load_record",
	"load_spec",
	"rescore",
	"run",
	"StudentResult",
	"TestSpec",
]
