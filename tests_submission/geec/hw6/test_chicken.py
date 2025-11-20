from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "chichen.py"
FUNC_NAME = "find_chicken_solutions"

@pytest.fixture(scope="function")
def student_function(get_function) -> Callable[[], list[tuple[int, int, int]]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)

@pytest.fixture(scope="function")
def student_result(student_function)-> list[tuple[int, int, int]]:
	"""
	获取学生提交的函数的结果
	"""
	return student_function()


def test_chicken_solution_count(student_result):
	"""
	测试解的总数是否为 4
	"""
	assert len(student_result) == 4, f"解的数量应为 4，但找到了 {len(student_result)} 个"


def test_chicken_solution_1_exists(student_result):
	"""
	测试解 (0, 25, 75) 是否存在
	"""
	solution = (0, 25, 75)
	assert solution in student_result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_2_exists(student_result):
	"""
	测试解 (4, 18, 78) 是否存在
	"""
	solution = (4, 18, 78)
	assert solution in student_result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_3_exists(student_result):
	"""
	测试解 (8, 11, 81) 是否存在
	"""
	solution = (8, 11, 81)
	assert solution in student_result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_4_exists(student_result):
	"""
	测试解 (12, 4, 84) 是否存在
	"""
	solution = (12, 4, 84)
	assert solution in student_result, f"预期解 {solution} 未在结果中找到"
