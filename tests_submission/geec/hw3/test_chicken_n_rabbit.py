# tests/test_chicken_rabbit.py
from typing import Callable, List, Tuple

import pytest

# --- 辅助函数，用于安全地加载模块和函数 ---

FILE_NAME = "Lab3_2"
FUNC_NAME_1 = "solve_chicken_rabbit"
FUNC_NAME_2 = "get_user_input"


@pytest.fixture(scope="function")
def student_function_solve_chicken_rabbit(get_function) -> Callable[[int, int], List[Tuple[int, int]]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_1, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_get_user_input(get_function) -> Callable[[int, int], List[Tuple[int, int]]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_2, FILE_NAME)


# --- 1. 测试核心逻辑函数：solve_chicken_rabbit ---

# 使用 pytest.mark.parametrize 为 solve_chicken_rabbit 提供多种测试场景
@pytest.mark.parametrize(
	"total_heads, total_feet, expected_solution",
	[
		# 测试用例 1: 经典的标准解
		(35, 94, [(23, 12)]),
		# 测试用例 2: 无解的情况
		(10, 31, []),
		# 测试用例 3: 边界情况 - 全是鸡
		(10, 20, [(10, 0)]),
		# 测试用例 4: 边界情况 - 全是兔
		(10, 40, [(0, 10)]),
		# 测试用例 5: 零输入的情况
		(0, 0, [(0, 0)]),
		# 测试用例 6: 数学上不可能的情况 (脚太少)
		(20, 30, []),
		# 测试用例 7: 多个解？(在此问题中不可能，但可用于验证逻辑)
		# (此用例省略，因为鸡兔同笼问题对于给定的头和脚只有唯一解或无解)
	],
)
def test_solve_chicken_rabbit(
	student_function_solve_chicken_rabbit, total_heads, total_feet, expected_solution
):
	"""
	对 solve_chicken_rabbit 函数进行参数化测试，覆盖多种情况。
	"""
	# 调用学生实现的函数
	actual_solution = student_function_solve_chicken_rabbit(total_heads, total_feet)

	# 断言：检查返回类型是否为列表
	assert isinstance(actual_solution, list), "返回值应为一个列表。"

	# 断言：检查结果是否与预期解完全一致
	# 使用 set 比较可以忽略解决方案的顺序（尽管此问题中只有一个解）
	assert set(actual_solution) == set(
		expected_solution
	), f"对于 heads={total_heads}, feet={total_feet}，计算结果不正确。"


# --- 2. 测试用户交互函数：get_user_input ---


def test_get_user_input_valid_input(student_function_get_user_input, monkeypatch):
	"""
	测试 get_user_input 函数能否正确处理有效的用户输入。
	我们使用 monkeypatch 来模拟用户的键盘输入。
	"""
	# 模拟用户先后输入 "35" 和 "94"
	user_inputs = "35\n94\n"
	monkeypatch.setattr("sys.stdin", iter(user_inputs.splitlines()))

	# 调用学生的函数，并捕获其返回值
	# 注意：这个测试依赖于学生在 get_user_input 函数中正确地 return 了获取到的值
	returned_heads, returned_feet = student_function_get_user_input()

	# 断言：检查函数是否正确返回了用户输入的值
	assert returned_heads == 35, "未能正确获取并返回头的数量。"
	assert returned_feet == 94, "未能正确获取并返回脚的数量。"


# (可选的高级测试)
def test_get_user_input_prints_solution(student_function_get_user_input, monkeypatch, capsys):
	"""
	(高级) 测试 get_user_input 是否在获取输入后，调用了核心函数并打印了正确结果。
	"""
	# 模拟用户输入
	user_inputs = "35\n94\n"
	monkeypatch.setattr("sys.stdin", iter(user_inputs.splitlines()))

	# 运行函数
	student_function_get_user_input()

	# 捕获打印到终端的输出
	captured = capsys.readouterr()

	# 断言：检查输出中是否包含了正确的结果 "23" 和 "12"
	# 这是一个灵活的检查，不强制要求固定的输出格式
	assert "23" in captured.out, "函数的打印输出中应包含鸡的数量 '23'。"
	assert "12" in captured.out, "函数的打印输出中应包含兔的数量 '12'。"
