from logging import error, info, warning
from pathlib import Path
from typing import List

import pytest
import typer

from scriptmark.utils import group_files_by_sid


class GraderDataPlugin:
	def __init__(self, student_data: dict):
		self.student_data = student_data

	def pytest_configure(self, config):
		"""
		这是 pytest 的一个钩子 (hook)，在测试配置阶段运行。
		我们将数据附加到 pytest 的 config 对象上，这样其他钩子就可以访问它。
		"""
		config.student_data_map = self.student_data


def run_tests(
	submissions_paths: List[Path] = typer.Argument(
		..., help="一个或多个学生代码所在的目录路径。", exists=True, file_okay=False
	),
	tests_dir: Path = typer.Option(
		...,
		"--tests-dir",
		"-t",
		help="包含 pytest 测试文件的目录。",
		exists=True,
		file_okay=False,
	),
	output_file: Path = typer.Option(
		"output/summary_report.xml", "--output", "-o", help="测试报告的路径与名字。"
	),
	timeout: int = typer.Option(10, help="每个测试用例的超时时间（秒）。"),
):
	student_files = group_files_by_sid(submissions_paths)

	info("Found {} student submissions.".format(len(student_files)))


	if output_file.suffix != ".xml":
		warning("测试报告文件名必须以 .xml 结尾。")
		output_file = output_file.with_suffix(".xml")

	# --- 2. Create results directory ---
	output_file.parent.mkdir(exist_ok=True)
	info(f"Test reports will be saved in '{output_file}'.")


	# 4. Run pytest ONCE for all students
	info("Starting a single pytest session for all students...")
	pytest_args = [
		str(tests_dir),
		f"--junitxml={output_file}",
		"-v",
		f"--timeout={timeout}",
	]

	data_plugin = GraderDataPlugin(student_files)

	info("Starting pytest session...")

	exit_code = pytest.main(pytest_args, plugins=[data_plugin])

	info("Pytest session finished.")
	info(f"Unified report saved to '[cyan]{output_file}[/cyan]'.")

	# 你可以根据退出码判断测试是否全部通过
	if exit_code == pytest.ExitCode.OK:
		info("[bold green]All tests passed for all students.[/bold green]")
	elif exit_code == pytest.ExitCode.TESTS_FAILED:
		warning("[bold yellow]Some tests failed.[/bold yellow]")
	else:
		error(f"[bold red]Pytest exited with code: {exit_code}[/bold red]")
