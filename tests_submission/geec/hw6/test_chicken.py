import pytest
from typing import List, Tuple, Set

# --- 测试配置 ---
FILE_NAME = "chichen.py"
FUNC_NAME = "find_chicken_solutions"


def test_chicken_solution_count(get_function):
	"""
	测试解的总数是否为 4
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func()
	assert len(result) == 4, f"解的数量应为 4，但找到了 {len(result)} 个"


def test_chicken_solution_1_exists(get_function):
	"""
	测试解 (0, 25, 75) 是否存在
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func()
	solution = (0, 25, 75)
	assert solution in result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_2_exists(get_function):
	"""
	测试解 (4, 18, 78) 是否存在
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func()
	solution = (4, 18, 78)
	assert solution in result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_3_exists(get_function):
	"""
	测试解 (8, 11, 81) 是否存在
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func()
	solution = (8, 11, 81)
	assert solution in result, f"预期解 {solution} 未在结果中找到"


def test_chicken_solution_4_exists(get_function):
	"""
	测试解 (12, 4, 84) 是否存在
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func()
	solution = (12, 4, 84)
	assert solution in result, f"预期解 {solution} 未在结果中找到"
