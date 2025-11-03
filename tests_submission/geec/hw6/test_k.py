import pytest
from typing import List

# --- 测试配置 ---
FILE_NAME = "k.py"
FUNC_NAME = "find_k_numbers"


class TestKNumbers:

	@pytest.fixture(scope="function", autouse=True)
	def setup_function(self, get_function):
		"""
		加载函数并运行，将结果存储起来以便于测试。
		"""
		self.find_k_func = get_function(FUNC_NAME, FILE_NAME)

		# 运行函数
		try:
			self.results = self.find_k_func(100000)
		except Exception as e:
			pytest.fail(f"调用 {FUNC_NAME}(100000) 失败: {e}")

		# 确保返回的是列表
		assert isinstance(self.results, list), f"{FUNC_NAME} 应返回 List，但返回了 {type(self.results)}"

		# 标准答案
		self.expected_k_numbers = [1, 81, 100, 2025, 3025, 9801]

	# --- 原子化测试 ---

	def test_k_numbers_count(self, get_function):
		"""
		测试 100000 以内的 K 数数量是否为 6
		"""
		assert len(self.results) == 6, f"K 数的数量应为 6，但找到了 {len(self.results)} 个"

	def test_k_number_1_exists(self, get_function):
		"""测试 K 数 1 是否存在"""
		assert 1 in self.results, "K 数 1 未在结果中找到"

	def test_k_number_81_exists(self, get_function):
		"""测试 K 数 81 是否存在"""
		assert 81 in self.results, "K 数 81 未在结果中找到"

	def test_k_number_100_exists(self, get_function):
		"""测试 K 数 100 是否存在"""
		assert 100 in self.results, "K 数 100 未在结果中找到"

	def test_k_number_2025_exists(self, get_function):
		"""测试 K 数 2025 是否存在"""
		assert 2025 in self.results, "K 数 2025 未在结果中找到"

	def test_k_number_3025_exists(self, get_function):
		"""测试 K 数 3025 是否存在"""
		assert 3025 in self.results, "K 数 3025 未在结果中找到"

	def test_k_number_9801_exists(self, get_function):
		"""测试 K 数 9801 是否存在"""
		assert 9801 in self.results, "K 数 9801 未在结果中找到"

	def test_k_numbers_are_all_correct(self, get_function):
		"""
		测试所有 K 数是否完全匹配 (忽略顺序)
		"""
		assert sorted(self.results) == sorted(self.expected_k_numbers), \
			f"找到的K数不正确。预期 {self.expected_k_numbers}，但得到 {sorted(self.results)}"
