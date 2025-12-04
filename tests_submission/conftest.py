# tests/conftest.py (Upgraded Version)
import builtins
import logging
import signal

import pytest
import importlib.util
import sys
from pathlib import Path
from typing import Any, Callable, List


@pytest.fixture(autouse=True)
def disable_input(monkeypatch):
	"""
	默认禁用 input() 函数。
	任何尝试调用 input() 的代码都会立即抛出 RuntimeError。
	防止自动化测试因等待用户输入而无限期挂起。
	"""

	def mock_input(prompt=""):
		raise RuntimeError(
			f"❌ Error: input() called during test.\n"
			f"Promt was: '{prompt}'\n"
			f"Please remove interactive input calls from your solution functions."
		)

	# 将内置的 input 替换为 mock_input
	monkeypatch.setattr(builtins, "input", mock_input)


@pytest.fixture(autouse=True)
def limit_execution_time():
	"""
	限制每个测试用例的执行时间。
	默认设置为 2 秒。作为防止 'except:' (裸except) 捕获 BaseException 的最后一道防线。
	"""
	# Windows 系统不支持 SIGALRM，直接跳过
	if not hasattr(signal, "SIGALRM"):
		yield
		return

	def handler(signum, frame):
		raise TimeoutError("❌ Test timed out! (Limit: 2s) - 可能存在死循环")

	# 注册信号处理器
	original_handler = signal.signal(signal.SIGALRM, handler)
	# 设置 2 秒超时
	signal.alarm(2)

	try:
		yield
	finally:
		# 取消闹钟并还原处理器
		signal.alarm(0)
		signal.signal(signal.SIGALRM, original_handler)


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
		if ends_with.endswith(".py"):
			ends_with = ends_with[:-3]
			logging.warning(f"Stripping '.py' suffix from <{ends_with}> for {self.sid}")

		# Check cache first
		if ends_with in self._loaded_modules:
			return self._loaded_modules[ends_with]

		# Find the file that matches the suffix

		file_path = None

		for p in self.files:
			if p.name.endswith(ends_with):
				file_path = p
				break

		if not file_path:
			for p in self.files:
				possible_strips = [p.name.rstrip("-1"), p.name.rstrip("-2"), p.name.rstrip("_1"), p.name.rstrip("_2"), p.name.rstrip(".1"), p.name.rstrip(".2")]
				for ps in possible_strips:
					if ps.endswith(ends_with):
						file_path = p
						logging.warning(f"Using Striped Name {ps} instead of <{p.name}> for {p.name}")
						break
				if file_path:
					break

		if not file_path:
			for p in self.files:
				in_stem = ends_with.split('.')[0]
				if in_stem in p.name:
					file_path = p
					logging.warning(f"Using <IN> {in_stem} instead of endswith <{ends_with}> for {p.name}")
					break

		if not file_path:
			pytest.fail(
				f"Could not find a file ending with '{ends_with}' for student {self.sid}."
			)

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
def student_submission(request, record_property):  # <-- 1. Add 'record_property' here
	"""
	This fixture is now a factory. It creates and returns a Submission object
	for each student, which the tests can then use to request modules.
	"""
	student_data = request.param
	student_id = student_data["sid"]  # <-- 2. Get the student ID

	record_property("student_id", student_id)

	# The rest of your function is perfect
	return Submission(sid=student_id, file_paths=student_data["files"])


def fake_input(*args, **kwargs):
	return 0


@pytest.fixture(scope="function")
def get_module(student_submission: Submission, monkeypatch) -> Callable:
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
		monkeypatch.setattr('builtins.input', fake_input)
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
