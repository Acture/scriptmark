import pytest
from typing import List, Tuple, Set

# --- 测试配置 ---
FILE_NAME = "chichen.py"
FUNC_NAME = "find_chicken_solutions"


class TestChickenProblem:

	# 声明一个类级别的 fixture，只运行一次
	@pytest.fixture(scope="class")
	def solve_once(self, get_function_from_class):
		"""
		在所有测试之前，只调用一次目标函数，并存储其结果。
		这可以防止在每个原子测试中都重新计算。
		"""
		# get_function_from_class 是 conftest.py 中定义的另一个（假设的）夹具
		# 如果你只有 get_function，你需要调整 conftest.py 或移除这个优化

		# 权宜之计：我们还是用 function scope 来加载，
		# 但把结果缓存在类变量里，虽然不那么 "pytestic"，但能工作

		# 为了兼容性，我们还是在 setup_function 中加载
		pass

	@pytest.fixture(scope="function", autouse=True)
	def setup_function(self, get_function):
		"""
		加载函数并运行，将结果转换为集合以便于测试。
		"""
		self.solve_func = get_function(FUNC_NAME, FILE_NAME)

		# 运行函数
		results = self.solve_func()

		# 确保返回的是列表
		assert isinstance(results, list), f"{FUNC_NAME} 应返回 List，但返回了 {type(results)}"

		# 转换为 Set[Tuple] 以便后续比较
		try:
			self.result_set = {tuple(item) for item in results}
		except TypeError:
			pytest.fail(f"{FUNC_NAME} 的返回值 {results} 无法转换为元组集合。")

	# --- 原子化测试 ---

	def test_chicken_solution_count(self, get_function):
		"""
		测试解的总数是否为 4
		"""
		assert len(self.result_set) == 4, f"解的数量应为 4，但找到了 {len(self.result_set)} 个"

	def test_chicken_solution_1_exists(self, get_function):
		"""
		测试解 (0, 25, 75) 是否存在
		"""
		solution = (0, 25, 75)
		assert solution in self.result_set, f"预期解 {solution} 未在结果中找到"

	def test_chicken_solution_2_exists(self, get_function):
		"""
		测试解 (4, 18, 78) 是否存在
		"""
		solution = (4, 18, 78)
		assert solution in self.result_set, f"预期解 {solution} 未在结果中找到"

	def test_chicken_solution_3_exists(self, get_function):
		"""
		测试解 (8, 11, 81) 是否存在
		"""
		solution = (8, 11, 81)
		assert solution in self.result_set, f"预期解 {solution} 未在结果中找到"

	def test_chicken_solution_4_exists(self, get_function):
		"""
		测试解 (12, 4, 84) 是否存在
		"""
		solution = (12, 4, 84)
		assert solution in self.result_set, f"预期解 {solution} 未在结果中找到"
