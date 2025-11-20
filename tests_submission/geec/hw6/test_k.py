from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "k.py"
FUNC_NAME = "find_k_numbers"


@pytest.fixture(scope="function")
def student_function(get_function) -> Callable[[int], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)

@pytest.fixture(scope="function")
def student_result(student_function)-> list[int]:
	"""
	获取学生提交的函数的结果
	"""
	return student_function(100000)


def test_k_numbers_count(student_result):
	"""
	测试 100000 以内的 K 数数量是否为 6
	"""
	assert len(student_result) == 6, f"K 数的数量应为 6，但找到了 {len(student_result)} 个"


def test_k_number_1_exists(student_result):
	"""测试 K 数 1 是否存在"""

	assert 1 in student_result, "K 数 1 未在结果中找到"


def test_k_number_81_exists(student_result):
	"""测试 K 数 81 是否存在"""
	assert 81 in student_result, "K 数 81 未在结果中找到"


def test_k_number_100_exists(student_result):
	"""测试 K 数 100 是否存在"""
	assert 100 in student_result, "K 数 100 未在结果中找到"


def test_k_number_2025_exists(student_result):
	"""测试 K 数 2025 是否存在"""
	assert 2025 in student_result, "K 数 2025 未在结果中找到"


def test_k_number_3025_exists(student_result):
	"""测试 K 数 3025 是否存在"""
	assert 3025 in student_result, "K 数 3025 未在结果中找到"


def test_k_number_9801_exists(student_result):
	"""测试 K 数 9801 是否存在"""
	assert 9801 in student_result, "K 数 9801 未在结果中找到"


def test_k_numbers_are_all_correct(student_result):
	"""
	测试所有 K 数是否完全匹配 (忽略顺序)
	"""
	expected_k_numbers = [1, 81, 100, 2025, 3025, 9801]
	assert sorted(student_result) == sorted(expected_k_numbers), \
		f"找到的K数不正确。预期 {expected_k_numbers}，但得到 {sorted(student_result)}"
