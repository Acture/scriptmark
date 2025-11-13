from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "similar.py"
FUNC_NAME = "are_words_similar"


@pytest.fixture(scope="function")
def student_function(get_function) -> Callable[[str], bool]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)


@pytest.mark.parametrize("word1, word2, expected", [
	# --- 基础测试 (来自作业要求) ---
	("listen", "silent", True),
	("listen", "silenta", False),

	# --- 更多正向测试 (True) ---
	("team", "meat", True),
	("restful", "fluster", True),
	("world", "olrwd", True),
	# 测试重复字母（集合应该只包含 'h', 'e', 'l', 'o'）
	("hello", "olelh", True),
	# 测试 'apple' {'a', 'p', 'l', 'e'} vs 'aple' {'a', 'p', 'l', 'e'}
	# 按照定义，它们是相似的
	("apple", "aple", True),
	("a", "a", True),

	# --- 更多反向测试 (False) ---
	("hello", "helo", False),  # 缺少 'l'
	("program", "function", False),
	("a", "b", False),

	# --- 边界情况测试 ---
	("", "", True),  # 两个空字符串（空集 == 空集）
	("abc", "", False),  # 一个为空
	("", "abc", False),  # 另一个为空
	("test", "testing", False),
])
def test_are_similar(student_function, word1, word2, expected):
	"""
	测试 are_similar 函数是否能正确判断两个词是否“相似”。
	"""
	assert student_function(word1, word2) == expected
