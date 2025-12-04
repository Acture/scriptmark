from pathlib import Path
from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "switch_case.py"
FUNC_NAME_1 = "switch_case"
FUNC_NAME_2 = "process_file"


@pytest.fixture(scope="function")
def student_function_switch_case(get_function) -> Callable[[str], str]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_1, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_process_file(get_function) -> Callable[[Path, Path], None]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_2, FILE_NAME)


@pytest.mark.parametrize("input_str, expected", [
	# --- 基础测试 (来自作业要求) ---
	("hello", "HELLO"),
	("world", "WORLD"),

	# --- 混合内容测试 ---
	("Hello, World!", "HELLO, WORLD!"),
	("Python 3.12 is Cool", "PYTHON 3.12 IS COOL"),

	# --- 无需转换的情况 ---
	("12345", "12345"),  # 纯数字
	("!@#$%", "!@#$%"),  # 纯标点
	("ALREADY UPPER", "ALREADY UPPER"),  # 已是大写
	("   \t\n", "   \t\n"),  # 空白字符

	# --- 边界情况 ---
	("", ""),  # 空字符串
	("a", "A"),  # 单字符
	("z", "Z"),  # 字母表边界
])
def test_switch_case_logic(student_function_switch_case, input_str, expected):
	"""
	测试 switch_case 函数的核心转换逻辑。
	"""
	result = student_function_switch_case(input_str)
	assert result == expected, f"输入 '{input_str}' 应该转换为 '{expected}'，但得到了 '{result}'"


@pytest.mark.parametrize("file_content, expected_content_in_file", [
	# 用例 1: 简单的小写转大写
	("abc", "ABC"),

	# 用例 2: 混合句子
	("Please convert this: abc -> ABC.", "PLEASE CONVERT THIS: ABC -> ABC."),

	# 用例 3: 多行文本
	("Line 1\nLine 2\nline 3", "LINE 1\nLINE 2\nLINE 3"),

	# 用例 4: 空文件
	("", ""),

	# 用例 5: 无需转换的特殊字符
	("123 + 456 = 579", "123 + 456 = 579"),
])
def test_process_files_success(student_function_process_file, tmp_path, file_content, expected_content_in_file):
	"""
	[参数化] 测试完整的文件读写流程。
	"""
	# 1. 准备环境
	input_file = tmp_path / "test_in.txt"
	output_file = tmp_path / "test_out.txt"

	# 写入模拟输入文件
	input_file.write_text(file_content, encoding="utf-8")

	# 2. 调用学生函数
	student_function_process_file(input_file, output_file)

	# 3. 验证结果
	assert output_file.exists(), "输出文件未被创建"
	actual_out = output_file.read_text(encoding="utf-8")
	assert actual_out == expected_content_in_file


@pytest.mark.parametrize("pre_existing_content, new_input_content, expected_output_content", [
	# 情况 1: 旧内容较短，新内容覆盖后变长 (常规覆盖)
	("old", "hello world", "HELLO WORLD"),

	# 情况 2: [关键] 旧内容很长，新内容很短 (检测是否正确截断文件)
	# 如果学生使用了 'r+' 且未 truncate，结果可能会是 "SHORT LONG CONTENT..."
	("VERY LONG OLD CONTENT THAT NEEDS TO BE REMOVED", "short", "SHORT"),

	# 情况 3: 覆盖为空 (检测是否能清空文件)
	("Some content", "", ""),
])
def test_process_files_overwrite(student_function_process_file, tmp_path, pre_existing_content, new_input_content, expected_output_content):
	"""
	[参数化] 测试文件覆盖功能。
	重点测试当输出文件已存在不同长度的内容时，是否能正确完全覆盖。
	"""
	input_file = tmp_path / "in.txt"
	output_file = tmp_path / "out.txt"

	# 1. 准备环境：写入新输入，并预置旧输出
	input_file.write_text(new_input_content, encoding="utf-8")
	output_file.write_text(pre_existing_content, encoding="utf-8")

	# 2. 执行学生代码
	student_function_process_file(input_file, output_file)

	# 3. 验证结果
	assert output_file.read_text(encoding="utf-8") == expected_output_content


@pytest.mark.parametrize("error_scenario_type", [
	"missing_file",  # 场景 1: 文件不存在
	"is_directory",  # 场景 2: 输入路径是一个目录 (应触发 IsADirectoryError/PermissionError -> OSError)
])
def test_process_files_io_errors(student_function_process_file, tmp_path, capsys, error_scenario_type):
	"""
	[参数化] 测试异常处理。
	验证函数能否捕获不同类型的 I/O 错误，并向 stderr 打印信息而不崩溃。
	"""
	input_path = tmp_path / "bad_input"
	output_path = tmp_path / "should_not_exist.txt"

	# 根据参数构建错误场景
	if error_scenario_type == "missing_file":
		# 不创建文件，保持不存在状态
		pass
	elif error_scenario_type == "is_directory":
		# 创建一个同名目录
		input_path.mkdir()

	# 执行并断言
	try:
		student_function_process_file(input_path, output_path)
	except OSError:
		pytest.fail(f"场景 '{error_scenario_type}' 下，函数未捕获 IOError/OSError，程序崩溃了。")

	# 验证
	assert not output_path.exists(), "发生输入错误时，不应创建输出文件"

	captured = capsys.readouterr()
	assert len(captured.err) > 0, f"场景 '{error_scenario_type}' 下，应向 sys.stderr 打印错误信息"
