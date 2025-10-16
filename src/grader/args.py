import argparse
from pathlib import Path
from typing import List, Optional

from pydantic import BaseModel


class Config(BaseModel):
	"""Configuration for the grader."""
	submissions_paths: List[Path]
	tests_dir: List[Path]
	output_dir: Path
	timeout: int
	roster: Optional[Path]


def parse_args() -> Config:
	"""Parses command-line arguments."""
	parser = argparse.ArgumentParser(
		description="A flexible auto-grader for Python assignments using pytest."
	)

	parser.add_argument(
		"submissions_paths",
		type=Path,
		nargs='+',
		help="Path to the directory of student submissions"
	)

	parser.add_argument(
		"-t", "--tests-dir",
		type=Path,
		required=True,
		nargs='+',
		help="Path to the directory containing pytest test files (default: tests)."
	)

	parser.add_argument(
		"-o", "--output-dir",
		type=Path,
		default=Path("output"),
		help="Directory to save the JUnit XML test results (default: results)."
	)

	parser.add_argument(
		"--timeout",
		type=int,
		default=10,
		help="Timeout in seconds for each individual test case (default: 10)."
	)

	parser.add_argument(
		"-r", "--roster",
		type=Path,
		default=None,
		help="Path to the roster CSV file."
	)

	return Config(**vars(parser.parse_args()))
