# tests/conftest.py (Upgraded Version)

import pytest
import importlib.util
import sys
from pathlib import Path
from typing import Any, Callable, List


def pytest_generate_tests(metafunc):
	"""
	动态生成测试。现在从 pytest 的 config 对象中直接读取数据。
	"""
	if "student_submission" in metafunc.fixturenames:
		# --- 核心改动在这里 ---
		# `metafunc.config` 就是上面 `pytest_configure` 操作过的 config 对象
		if not hasattr(metafunc.config, "student_data_map"):
			pytest.fail(
				"Grader data was not passed to pytest correctly. Check the plugin."
			)

		student_files_map = metafunc.config.student_data_map

		# 后续的参数化逻辑完全不变
		params = [
			{"sid": sid, "files": files} for sid, files in student_files_map.items()
		]

		metafunc.parametrize(
			"student_submission", params, indirect=True, ids=student_files_map.keys()
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
			pytest.fail(
				f"For student '{self.sid}', no file ending with '{ends_with}' was found."
			)

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
	return Submission(sid=student_data["sid"], file_paths=student_data["files"])


@pytest.fixture(scope="function")
def get_module(student_submission: Submission) -> Callable:
	"""
	"模块工厂夹具" (Module Factory Fixture)

	这个夹具返回一个内部辅助函数 (`_getter`)。
	测试用例可以调用 `_getter(file_ends_with: str)`
	来动态加载并返回任何所需的模块。
	"""

	def _getter(file_ends_with: str) -> Any:
		"""
		这是实际返回给测试用例的辅助函数。
		它使用 Submission 管理器来安全地加载模块。
		"""
		# Submission.get_module 已经包含了所有错误处理
		# (例如：未找到文件, 找到多个文件, 导入失败)
		return student_submission.get_module(ends_with=file_ends_with)

	# Fixture 返回这个 _getter 函数
	return _getter


@pytest.fixture(scope="function")
def get_function(get_module: Callable) -> Callable:
	"""
	"函数工厂夹具" (Function Factory Fixture)

	这个夹具返回一个内部辅助函数 (`_getter`)。
	测试用例可以调用 `_getter(func_name: str, file_ends_with: str)`
	来动态地从任何文件中加载任何函数。

	这个夹具依赖于上面的 `get_module` 工厂。
	"""

	def _getter(func_name: str, file_ends_with: str) -> Callable:
		"""
		这是实际返回给测试用例的辅助函数。

		Args:
		        func_name: 要查找的函数名 (例如 "calculate_sum")
		        file_ends_with: 要加载的文件后缀 (例如 "Lab3_task1.py")
		"""

		# 步骤 1: 使用 get_module 工厂获取正确的模块
		# get_module 是从同名夹具注入的 _getter 函数
		module = get_module(file_ends_with)

		# 步骤 2: 从加载的模块中安全地获取函数
		if not hasattr(module, func_name):
			pytest.skip(
				f"Function '{func_name}' not found in module '{file_ends_with}'."
			)

		func = getattr(module, func_name)

		if not callable(func):
			pytest.fail(
				f"Found '{func_name}' in '{file_ends_with}', but it is not a function.",
				pytrace=False,
			)

		return func

	# Fixture 返回这个 _getter 函数
	return _getter
