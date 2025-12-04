from pathlib import Path
from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "Leap.py"
FUNC_NAME_1 = "isLeap"


@pytest.fixture(scope="function")
def student_function_isLeap(get_function) -> Callable[[str], str]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_1, FILE_NAME)


@pytest.mark.parametrize("year, expected", [
	# Case 1: 能被400整除 (世纪闰年) -> True
	(2000, True),
	(1600, True),
	
	# Case 2: 能被4整除但不能被100整除 (普通闰年) -> True
	(2024, True),
	(2004, True),
	(1996, True),
	
	# Case 3: 能被100整除但不能被400整除 (世纪平年) -> False
	(1900, False),
	(1700, False),
	(2100, False),
	
	# Case 4: 不能被4整除 (普通平年) -> False
	(2023, False),
	(2025, False),
	(1, False)
])
def test_isLeap(student_function_isLeap, year, expected):
	"""
	验证 isLeap 函数是否正确遵循公历置闰规则：
	四年一闰，百年不闰，四百年再闰。
	"""
	assert student_function_isLeap(year) == expected
