import pytest

# --- 测试配置 ---
# 假设函数在 Lab4.py 中
FILE_NAME = "Lab4.py"
# 假设要测试的函数名
FUNC_NAME = "translate_number_to_text"
# 模板中提供的固定错误消息
EXPECTED_ERROR_MSG = "错误：输入内容必须全部为数字。"


@pytest.mark.describe(f"测试 {FUNC_NAME} 函数 ({FILE_NAME})")
class TestTranslationLogic:
	"""
	本测试套件不关心 CHAR_MAP 的 *内容*，
	只测试学生的 *翻译逻辑* 和 *错误处理* 是否正确。
	"""

	@pytest.fixture(scope="function", autouse=True)
	def setup_function(self, get_function):
		"""
		在所有测试开始前，使用 conftest.py 中的 'get_function' 工厂，
		获取一次学生编写的 translate_number_to_text 函数。
		"""
		# get_function(func_name, file_ends_with)
		self.translate_func = get_function(FUNC_NAME, FILE_NAME)

	# --- 测试1：输入验证 ---

	def test_invalid_input_letters(self):
		"""测试：无效输入（包含字母）应返回固定错误"""
		assert self.translate_func("123a45") == EXPECTED_ERROR_MSG

	def test_invalid_input_symbols(self):
		"""测试：无效输入（包含符号）应返回固定错误"""
		assert self.translate_func("9-0") == EXPECTED_ERROR_MSG

	def test_invalid_input_spaces(self):
		"""测试：无效输入（包含空格）应返回固定错误"""
		assert self.translate_func("1 2 3") == EXPECTED_ERROR_MSG

	def test_invalid_input_empty(self):
		"""测试：无效输入（空字符串）应返回固定错误"""
		assert self.translate_func("") == EXPECTED_ERROR_MSG

	# --- 测试2：核心逻辑（拼接）---

	def test_concatenation_logic(self):
		"""
		测试核心拼接逻辑： T("12") 必须等于 T("1") + T("2")
		这个测试与学生定义的 CHAR_MAP 内容无关，是纯粹的逻辑验证。
		"""
		try:
			result_1 = self.translate_func("1")
			result_2 = self.translate_func("2")
			result_12 = self.translate_func("12")

			# 核心断言
			assert result_12 == result_1 + result_2, "翻译逻辑错误： T('12') 的结果不等于 T('1') + T('2')"

		except Exception as e:
			pytest.fail(f"测试拼接逻辑时发生意外错误（是否遗漏了 '1' 或 '2' 的映射？）： {e}")

	def test_repetition_logic(self):
		"""
		测试核心拼接逻辑： T("33") 必须等于 T("3") + T("3")
		"""
		try:
			result_3 = self.translate_func("3")
			result_33 = self.translate_func("33")

			assert result_33 == result_3 + result_3, "翻译逻辑错误： T('33') 的结果不等于 T('3') + T('3')"

		except Exception as e:
			pytest.fail(f"测试重复逻辑时发生意外错误（是否遗漏了 '3' 的映射？）： {e}")

	# --- 测试3：CHAR_MAP 完整性 ---

	def test_all_digits_are_mapped(self):
		"""
		测试：CHAR_MAP 是否为 '0' 到 '9' 都提供了映射。
		如果学生遗漏了任何一个键，此测试将因 KeyError 而失败。
		"""
		all_digits = "0123456789"
		try:
			# 我们只需要调用函数。如果任何键（'0'...'9'）缺失，
			# 学生的字典查找 `CHAR_MAP[ch]` 会抛出 KeyError，
			# Pytest 会自动捕获该异常并使测试失败。
			translation = self.translate_func(all_digits)

			# 额外检查：确保返回的是一个非空的字符串
			assert translation is not None, "函数不应返回 None"
			assert isinstance(translation, str), "函数应返回一个字符串 (str)"
			assert len(translation) > 0, "翻译 '0123456789' 不应返回空字符串"

		except KeyError as e:
			pytest.fail(f"CHAR_MAP 字典不完整。学生遗漏了数字 {e} 的映射。")
		except Exception as e:
			pytest.fail(f"翻译 '0123456789' 时发生意外错误: {e}")

	# --- 测试4：唯一性（您的要求）---

	def test_different_inputs_produce_different_outputs(self):
		"""
		测试（您的要求）：不同输入应产生不同输出。
		这鼓励学生为每个数字使用唯一的词。
		"""
		try:
			result_5 = self.translate_func("5")
			result_6 = self.translate_func("6")

			# 这个断言检查的是学生的 *数据* (CHAR_MAP) 质量
			assert result_5 != result_6, "任务要求：请为 '5' 和 '6' 设置 *不同* 的词组"

			result_8 = self.translate_func("8")
			result_9 = self.translate_func("9")
			assert result_8 != result_9, "任务要求：请为 '8' 和 '9' 设置 *不同* 的词组"

		except Exception as e:
			pytest.fail(f"测试唯一性时发生意外错误（是否遗漏了 '5', '6', '8' 或 '9' 的映射？）： {e}")
