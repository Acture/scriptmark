# tests/conftest.py (Upgraded Version)

import pytest
import importlib.util
import sys
from pathlib import Path
from typing import List


def pytest_generate_tests(metafunc):
	"""
	动态生成测试。现在从 pytest 的 config 对象中直接读取数据。
	"""
	if 'student_submission' in metafunc.fixturenames:

		# --- 核心改动在这里 ---
		# `metafunc.config` 就是上面 `pytest_configure` 操作过的 config 对象
		if not hasattr(metafunc.config, "student_data_map"):
			pytest.fail("Grader data was not passed to pytest correctly. Check the plugin.")

		student_files_map = metafunc.config.student_data_map

		# 后续的参数化逻辑完全不变
		params = [
			{"sid": sid, "files": files}
			for sid, files in student_files_map.items()
		]

		metafunc.parametrize(
			"student_submission",
			params,
			indirect=True,
			ids=student_files_map.keys()
		)


# --- The NEW Submission Manager Class ---
class Submission:
	"""A manager object for a single student's submission files."""

	def __init__(self, sid: str, file_paths: List[str]):
		self.sid = sid
		self.files = [Path(p) for p in file_paths]
		self._loaded_modules = {}  # Cache to avoid re-importing

	def get_module(self, ends_with: str):
		"""
		Finds, imports, and returns a module from a file ending with a specific suffix.
		"""
		# Check cache first
		if ends_with in self._loaded_modules:
			return self._loaded_modules[ends_with]

		# Find the file that matches the suffix
		found_files = [p for p in self.files if p.name.endswith(ends_with)]

		# --- Crucial Error Handling ---
		if not found_files:
			pytest.fail(f"For student '{self.sid}', no file ending with '{ends_with}' was found.")

		if len(found_files) > 1:
			pytest.fail(
				f"For student '{self.sid}', multiple files found ending with '{ends_with}': "
				f"{[p.name for p in found_files]}. Test is ambiguous."
			)

		file_path = found_files[0]
		module_name = file_path.stem

		# --- Dynamic Import Logic ---
		spec = importlib.util.spec_from_file_location(module_name, file_path)
		if not spec:
			pytest.fail(f"Could not create module spec for {file_path}")

		module = importlib.util.module_from_spec(spec)
		sys.modules[module_name] = module

		try:
			spec.loader.exec_module(module)
		except Exception as e:
			pytest.fail(
				f"Failed to import code from '{file_path.name}' for student {self.sid}. "
				f"Error: {e}"
			)

		# Cache the successfully loaded module and return it
		self._loaded_modules[ends_with] = module
		return module


@pytest.fixture
def student_submission(request):
	"""
	This fixture is now a factory. It creates and returns a Submission object
	for each student, which the tests can then use to request modules.
	"""
	student_data = request.param
	return Submission(sid=student_data['sid'], file_paths=student_data['files'])
