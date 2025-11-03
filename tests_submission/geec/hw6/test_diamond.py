import pytest
from typing import Optional

# --- 测试配置 ---
FILE_NAME = "diamond.py"
FUNC_NAME = "generate_diamond_string"


class TestDiamond:

	@pytest.fixture(scope="function", autouse=True)
	def setup_function(self, get_function):
		"""
		加载目标函数。
		"""
		self.diamond_func = get_function(FUNC_NAME, FILE_NAME)

	def test_diamond_n_1(self, get_function):
		"""
		测试 n=1 的情况 (最小菱形)
		"""
		# 预期输出：一个星号。
		expected_output = "*"

		result = self.diamond_func(1)

		# 检查返回的是否是字符串
		assert isinstance(result, str), f"{FUNC_NAME}(1) 应返回 str，但返回了 {type(result)}"
		# 检查内容，用 strip() 忽略两端空白
		assert result.strip() == expected_output, "n=1 时，输出应为 '*'"

	def test_diamond_n_2(self, get_function):
		"""
		测试 n=2 的情况 (总行数 2*2-1 = 3)
		"""
		# 预期输出：
		#  *
		# * * *
		#  *
		expected_output = (
			" * \n"
			"* * *\n"
			" * "
		)

		result = self.diamond_func(2)
		assert isinstance(result, str), f"{FUNC_NAME}(2) 应返回 str，但返回了 {type(result)}"
		assert result.strip() == expected_output.strip(), "n=2 时，菱形形状或间距错误"

	def test_diamond_n_3(self, get_function):
		"""
		测试 n=3 的情况 (总行数 2*3-1 = 5)
		"""
		# 预期输出：
		#   *
		#  * *
		# * * *
		#  * *
		#   *
		expected_output = (
			"  * \n"
			" * * \n"
			"* * *\n"
			" * * \n"
			"  * "
		)

		result = self.diamond_func(3)
		assert isinstance(result, str), f"{FUNC_NAME}(3) 应返回 str，但返回了 {type(result)}"
		assert result.strip() == expected_output.strip(), "n=3 时，菱形形状或间距错误"

	def test_diamond_invalid_n_0(self, get_function):
		"""
		测试 n=0 (无效输入，因为要求是 '正整数')
		"""
		# 预期：返回 None
		assert self.diamond_func(0) is None, f"{FUNC_NAME}(0) 应返回 None"

	def test_diamond_invalid_n_negative(self, get_function):
		"""
		测试 n=-2 (无效输入)
		"""
		# 预期：返回 None
		assert self.diamond_func(-2) is None, f"{FUNC_NAME}(-2) 应返回 None"
