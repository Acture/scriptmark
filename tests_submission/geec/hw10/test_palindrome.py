from pathlib import Path
from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "Palindrome.py"
FUNC_NAME_1 = "f1"
FUNC_NAME_2 = "f2"

@pytest.fixture(scope="function")
def student_function_f1(get_function) -> Callable[[str], str]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_1, FILE_NAME)

@pytest.fixture(scope="function")
def student_function_f2(get_function) -> Callable[[str], str]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_2, FILE_NAME)


@pytest.mark.parametrize("input_str", [
    "abcd",
    "x",
    "ab",
    "123",
    "race",
    ""
])
def test_f1_relaxed_properties(student_function_f1, input_str):
	"""
	放松的测试策略：关注属性而非具体实现。

	允许以下任何一种实现通过：
	1. Overlap (题目示例): 'abcd' -> 'abcdcba'
	2. Full Mirror (更长): 'abcd' -> 'abcddcba'
	3. Minimal (最短): 'aba' -> 'aba' (如果实现够智能)
	"""
	# 执行函数
	result = student_function_f1(input_str)
	
	# 属性 1: 结果必须包含原字符串作为前缀
	# (即我们是通过在后面追加字符变成回文的，而不是修改前面)
	assert result.startswith(input_str), \
		f"Result '{result}' should start with original '{input_str}'"
	
	# 属性 2: 结果必须是回文
	# (这是 f1 的核心定义)
	assert result == result[::-1], \
		f"Result '{result}' is not a palindrome"
	
	expected_max_len = 2 * len(input_str)
	assert len(input_str) <= len(result) <= expected_max_len


@pytest.mark.parametrize("input_str, expected", [
	# True Cases: 标准回文
	("abba", True),
	("abcba", True),
	("level", True),
	("a", True),  # 单字符通常视为回文
	("", True),  # 空字符串通常视为回文
	
	# False Cases: 非回文
	("abcd", False),
	("ab", False),
	("abab", False),  # 重复但非对称
])
def test_f2_verification(student_function_f2, input_str, expected):
	"""
	验证 f2 是否能准确判断字符串为回文 (正读反读一致)。
	"""
	assert student_function_f2(input_str) == expected