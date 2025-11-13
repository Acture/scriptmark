from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "scan.py"
FUNC_NAME = "calculate_scan_effect"


@pytest.fixture(scope="function")
def student_function(get_function)->Callable[[str], str]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)

def _generate_expected_scan(text: str) -> str:
	"""
	一个辅助函数，用于生成正确的、符合作业要求的预期输出字符串。
	这使得测试用例本身保持干净。
	"""
	n = len(text)
	if n <= 1:
		# 如果是单个字母或空字符串，没有扫描动画，只有分隔符
		return "-----"

	left_scan_lines = []
	# 循环 (n-1) 次
	for i in range(n - 1):
		# 索引从 n-1 (倒数第1个) 到 1 (倒数第 n-1 个)
		split_point = n - 1 - i
		line = text[:split_point] + " " + text[split_point:]
		left_scan_lines.append(line)

	# “向右扫描”是“向左扫描”的逆序
	right_scan_lines = list(reversed(left_scan_lines))

	# 将所有部分组合成一个单一的、用 \n 分隔的字符串
	all_lines = left_scan_lines + ["-----"] + right_scan_lines

	return "\n".join(all_lines)


@pytest.mark.parametrize("input_word", [
	"Test",
	"Hi",
	"Python",
	# 测试作业描述中的例子
	"University",
])
def test_scan_animation_normal(student_function, input_word):
	"""
	测试标准单词的扫描动画。
	"""
	expected_output = _generate_expected_scan(input_word)
	actual_output = student_function(input_word)

	# 比较预期和实际的多行字符串
	assert actual_output == expected_output


def test_scan_animation_edge_cases(student_function):
	"""
	测试边界情况：单个字母或空字符串。
	"""
	# 单个字母
	expected_single = _generate_expected_scan("A")
	assert student_function("A") == expected_single
	assert expected_single == "-----"  # 确认辅助函数逻辑

	# 空字符串
	expected_empty = _generate_expected_scan("")
	assert student_function("") == expected_empty
	assert expected_empty == "-----"  # 确认辅助函数逻辑
