import csv
import json
import xml.etree.ElementTree as ET
from collections import defaultdict
from dataclasses import field
from enum import Enum, StrEnum
from logging import error, info, warn
from pathlib import Path
import argparse
import re
from typing import Dict, List, Optional

import rich
import typer
from pydantic import BaseModel, computed_field
from rich.console import Console
from rich.table import Table
from rich.panel import Panel
from rich.syntax import Syntax


class FailureDetail(BaseModel):
	test_name: str
	message: str
	details: str


class TestStatus(StrEnum):
	PASSED = "PASSED"
	FAILED = "FAILED"


class PerTestResult(BaseModel):
	"""Represents a student's result on a single test function."""
	test_name: str
	# We only need to store the total runs and the failure details.
	total_runs: int
	failures_details: List[FailureDetail] = []

	# Let the model compute derived values automatically!
	@computed_field
	@property
	def failures_count(self) -> int:
		return len(self.failures_details)

	@computed_field
	@property
	def passed_count(self) -> int:
		return self.total_runs - self.failures_count

	@computed_field
	@property
	def status(self) -> TestStatus:
		return TestStatus.FAILED if self.failures_count > 0 else TestStatus.PASSED

	@computed_field
	@property
	def pass_rate(self) -> float:
		if self.total_runs == 0:
			return 0.0
		return (self.passed_count / self.total_runs) * 100


class SummaryReport(BaseModel):
	"""Represents the complete report for a single student."""
	student_id: str
	student_name: Optional[str] = None
	# This is the core data: a list of results for each test.
	per_test_results: List[PerTestResult]

	# --- Let this model compute its own overall summary ---
	@computed_field
	@property
	def total_tests(self) -> int:
		return sum(test.total_runs for test in self.per_test_results)

	@computed_field
	@property
	def passed_count(self) -> int:
		return sum(test.passed_count for test in self.per_test_results)

	@computed_field
	@property
	def failed_count(self) -> int:
		return sum(test.failures_count for test in self.per_test_results)

	@computed_field
	@property
	def status(self) -> TestStatus:
		return TestStatus.FAILED if self.failed_count > 0 else TestStatus.PASSED

	@computed_field
	@property
	def pass_rate(self) -> float:
		if self.total_tests == 0:
			return 0.0
		return (self.passed_count / self.total_tests) * 100


def parse_unified_xml(file_path: Path) -> Dict[str, SummaryReport]:
	"""
	Parses the unified XML report and constructs a dictionary of SummaryReport Pydantic models.
	"""
	try:
		tree = ET.parse(file_path)
		root = tree.getroot()

		# 1. Use a nested defaultdict to gather the raw "source of truth" data.
		# Structure: student_id -> test_name -> { "total_runs": int, "failures_details": list }
		student_data = defaultdict(lambda: defaultdict(lambda: {
			"total_runs": 0,
			"failures_details": []
		}))

		for testcase in root.findall('.//testcase'):
			name_attr = testcase.attrib.get('name', '')
			match = re.search(r'\[(.*?)\]', name_attr)
			if not match: continue

			student_id = match.group(1)
			test_name = name_attr.split('[')[0]

			# 2. Populate the raw data structure.
			student_data[student_id][test_name]["total_runs"] += 1

			failure_node = testcase.find('failure') or testcase.find('error') or testcase.find('skipped')
			if failure_node is not None:
				traceback = failure_node.text.strip() if failure_node.text else ""
				important_lines = [line for line in traceback.split('\n') if line.strip().startswith('E ')]
				details = "\n".join(important_lines) if important_lines else traceback
				student_data[student_id][test_name]["failures_details"].append(FailureDetail(
					test_name=test_name,
					message=failure_node.attrib.get('message', 'No message'),
					details=details,
				))

		# 3. Assemble the final Pydantic models from the raw data.
		final_reports = {}
		for sid, tests in student_data.items():
			per_test_results_list = []
			for test_name, data in tests.items():
				# Create the inner PerTestResult object.
				per_test_results_list.append(PerTestResult(
					test_name=test_name,
					total_runs=data["total_runs"],
					failures_details=data["failures_details"]
				))

			# Create the outer SummaryReport object. Pydantic handles the rest!
			final_reports[sid] = SummaryReport(
				student_id=sid,
				per_test_results=per_test_results_list
			)
		return final_reports

	except ET.ParseError as e:
		error(f"Error: Could not parse XML file '{file_path}'. Reason: {e}", err=True)
		return {}


def archive_result(archive_path: Path, result: Dict[str, SummaryReport]):
	"""Archives the summary reports to JSON or a detailed, per-test CSV file."""

	if archive_path.suffix == ".json":
		json_data = {sid: report.model_dump(mode='json') for sid, report in result.items()}
		with open(archive_path, 'w', encoding='utf-8') as f:
			# We dump the already-serialized string data.
			json.dump(json_data, f, indent=2)
		info(f"📈 Results archived to JSON: [cyan]{archive_path}[/cyan]")

	elif archive_path.suffix == "csv":
		# For CSV, we create a more detailed, granular report.
		with open(archive_path, 'w', newline='', encoding='utf-8') as f:
			writer = csv.writer(f)
			# 1. Define the new, more detailed header.
			writer.writerow([
				"student_name", "student_id", "test_name", "status", "passed", "failed",
				"total_runs", "pass_rate", "failure_messages"
			])
			# 2. Loop through each student, and then through each of their test results.
			for sid, report in sorted(result.items()):
				for test_result in report.per_test_results:
					failure_msgs = "; ".join([f.message for f in test_result.failures_details])
					writer.writerow([
						sid,
						test_result.test_name,
						test_result.status.value,
						test_result.passed_count,
						test_result.failures_count,
						test_result.total_runs,
						f"{test_result.pass_rate:.2f}",
						failure_msgs
					])
		info(f"📊 Granular results archived to CSV: [cyan]{archive_path}[/cyan]")


def load_roster(roster_path: Path) -> Dict[str, str]:
	"""
	Loads a roster CSV file into a dictionary mapping student_id to student_name.
	Assumes CSV format: student_id,student_name
	"""
	if not roster_path:
		return {}

	roster_map = {}
	try:
		with open(roster_path, 'r', encoding='utf-8-sig') as f:  # 'utf-8-sig' handles BOM
			reader = csv.reader(f)
			next(reader, None)
			for row in reader:
				student_name = row[0].strip()
				student_id = row[2].strip()
				if student_id:
					roster_map[student_id] = student_name
		info(f"✅ Roster loaded successfully with [bold]{len(roster_map)}[/bold] entries.")
		return roster_map
	except FileNotFoundError:
		error(f"[bold red]Error: Roster file not found at '{roster_path}'[/bold red]")
		return {}
	except Exception as e:
		error(f"[bold red]Error parsing roster file '{roster_path}': {e}[/bold red]")
		return {}


def generate_summary(
	paths: List[Path] = typer.Argument(
		..., help="One or more summary_report.xml files or directories containing them.", exists=True
	),
	roster: Path = typer.Option(
		None, "--roster", "-r",
		help="Path to a CSV roster file (student_id,student_name) to look up names.",
		exists=True, dir_okay=False, readable=True
	),
	archive_dir: Path = typer.Option(
		None, "--archive", "-a", help="Archive the combined results to the specified dir."
	),
	archive_format: str = typer.Option(
		"csv", "--format", "-f", help="Archive format: 'json' or 'csv'.", case_sensitive=False
	)
):
	"""
	Parses one or more pytest XML reports, displays a summary, and optionally archives the results.
	"""

	roster_map = load_roster(roster)

	# 1. Find all XML files first.
	xml_files_to_parse = []
	for path in paths:
		for f in path.glob("*.xml"):
			xml_files_to_parse.append(f)

	if not xml_files_to_parse:
		error("[bold red]Error: No 'summary_report.xml' files found in the specified paths.[/bold red]")
		return

	# 2. Parse all files and merge them into a single master dictionary.
	for xml_file in xml_files_to_parse:
		info(f"Parsing report: [dim]{xml_file}[/dim]")
		result = parse_unified_xml(xml_file)

		summary_table = Table(title="🏆 Pytest Grading Summary 🏆")
		summary_table.add_column("Student Name", style="white")  # New Column
		summary_table.add_column("Student ID", justify="right", style="cyan", no_wrap=True)
		summary_table.add_column("Status", style="magenta")
		summary_table.add_column("Passed", justify="right")
		summary_table.add_column("Failed", justify="right")
		summary_table.add_column("Total", justify="right")
		summary_table.add_column("Pass Rate", justify="right")

		for student_id, report in sorted(result.items()):
			status_color = "green" if report.status == TestStatus.PASSED else "red"
			report.student_name = roster_map.get(student_id, "N/A")

			summary_table.add_row(
				report.student_name,
				report.student_id,
				f"[{status_color}]{report.status.value}[/{status_color}]",
				str(report.passed_count),  # Use the computed field
				str(report.failed_count),  # Use the computed field
				str(report.total_tests),  # Use the computed field
				f"{report.pass_rate:.2f}%"  # Use the computed field
			)
		rich.print(summary_table)

		# --- Display Failure Panels ---
		failed_students = {sid: r for sid, r in result.items() if r.status == TestStatus.FAILED}
		if failed_students:
			rich.print("\n\n--- [bold red]Detailed Failure Reports[/bold red] ---")
			for sid, report in sorted(failed_students.items()):
				# Loop through the per_test_results to show failures
				panel_content = ""
				for test_result in report.per_test_results:
					if test_result.status == TestStatus.FAILED:
						panel_content += f"[bold yellow]Failed Test:[/] [bold]{test_result.test_name}[/bold]\n"
						for failure in test_result.failures_details:
							panel_content += f"[dim]Message: {failure.message}[/dim]\n\n"
							syntax = Syntax(failure.details, "python", theme="solarized-dark", line_numbers=True)

							temp_console = Console(record=True, width=120)

							temp_console.print(syntax)

							rendered_syntax = temp_console.export_text()

							panel_content += rendered_syntax
				panel = Panel(
					panel_content.strip(),
					title=f"Failure Report for [bold magenta]{report.student_id}[/bold magenta]",
					border_style="red",
					title_align="left"

				)

				rich.print(panel)
		if archive_dir:
			archive_path = archive_dir / f"archive_{xml_file.stem}.{archive_format}"
			archive_result(archive_path, result)
