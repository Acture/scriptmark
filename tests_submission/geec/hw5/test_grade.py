import pytest

# --- 测试配置 ---
FILE_NAME = "Lab5_2.py"
FUNC_NAME = "calculate_grade"


@pytest.mark.describe(f"测试 {FUNC_NAME} 函数 ({FILE_NAME})")
class TestGradeLogic:

	@pytest.fixture(scope="function", autouse=True)
	def setup_function(self, get_function):
		"""
		加载目标函数。
		"""
		self.grade_func = get_function(FUNC_NAME, FILE_NAME)

	# --- 1. "A" 级别测试 ---
	def test_grade_A_100(self):
		assert self.grade_func(100.0) == "A", "calculate_grade(100.0) 应返回 'A'"

	def test_grade_A_90(self):
		assert self.grade_func(90.0) == "A", "calculate_grade(90.0) 应返回 'A'"

	def test_grade_A_95(self):
		assert self.grade_func(95.0) == "A", "calculate_grade(95.0) 应返回 'A'"

	def test_grade_A_overflow_101(self):
		assert self.grade_func(101.0) == "A", "calculate_grade(101.0) 应返回 'A'"

	def test_grade_A_overflow_200(self):
		assert self.grade_func(200.0) == "A", "calculate_grade(200.0) 应返回 'A'"

	# --- 2. "A-" 级别测试 ---
	def test_grade_A_minus_89_9(self):
		assert self.grade_func(89.9) == "A-", "calculate_grade(89.9) 应返回 'A-'"

	def test_grade_A_minus_85(self):
		assert self.grade_func(85.0) == "A-", "calculate_grade(85.0) 应返回 'A-'"

	def test_grade_A_minus_87_5(self):
		assert self.grade_func(87.5) == "A-", "calculate_grade(87.5) 应返回 'A-'"

	# --- 3. "B+" 级别测试 ---
	def test_grade_B_plus_84_9(self):
		assert self.grade_func(84.9) == "B+", "calculate_grade(84.9) 应返回 'B+'"

	def test_grade_B_plus_82(self):
		assert self.grade_func(82.0) == "B+", "calculate_grade(82.0) 应返回 'B+'"

	# --- 4. "B" 级别测试 ---
	def test_grade_B_81_9(self):
		assert self.grade_func(81.9) == "B", "calculate_grade(81.9) 应返回 'B'"

	def test_grade_B_78(self):
		assert self.grade_func(78.0) == "B", "calculate_grade(78.0) 应返回 'B'"

	# --- 5. "B-" 级别测试 ---
	def test_grade_B_minus_77_9(self):
		assert self.grade_func(77.9) == "B-", "calculate_grade(77.9) 应返回 'B-'"

	def test_grade_B_minus_75(self):
		assert self.grade_func(75.0) == "B-", "calculate_grade(75.0) 应返回 'B-'"

	# --- 6. "C+" 级别测试 ---
	def test_grade_C_plus_74_9(self):
		assert self.grade_func(74.9) == "C+", "calculate_grade(74.9) 应返回 'C+'"

	def test_grade_C_plus_71(self):
		assert self.grade_func(71.0) == "C+", "calculate_grade(71.0) 应返回 'C+'"

	# --- 7. "C" 级别测试 ---
	def test_grade_C_70_9(self):
		assert self.grade_func(70.9) == "C", "calculate_grade(70.9) 应返回 'C'"

	def test_grade_C_66(self):
		assert self.grade_func(66.0) == "C", "calculate_grade(66.0) 应返回 'C'"

	# --- 8. "C-" 级别测试 ---
	def test_grade_C_minus_65_9(self):
		assert self.grade_func(65.9) == "C-", "calculate_grade(65.9) 应返回 'C-'"

	def test_grade_C_minus_62(self):
		assert self.grade_func(62.0) == "C-", "calculate_grade(62.0) 应返回 'C-'"

	# --- 9. "D" 级别测试 ---
	def test_grade_D_61_9(self):
		assert self.grade_func(61.9) == "D", "calculate_grade(61.9) 应返回 'D'"

	def test_grade_D_60(self):
		assert self.grade_func(60.0) == "D", "calculate_grade(60.0) 应返回 'D'"

	# --- 10. "F" 级别测试 ---
	def test_grade_F_59_9(self):
		assert self.grade_func(59.9) == "F", "calculate_grade(59.9) 应返回 'F'"

	def test_grade_F_0(self):
		assert self.grade_func(0.0) == "F", "calculate_grade(0.0) 应返回 'F'"

	def test_grade_F_30(self):
		assert self.grade_func(30.0) == "F", "calculate_grade(30.0) 应返回 'F'"

	def test_grade_F_underflow_negative_0_1(self):
		assert self.grade_func(-0.1) == "F", "calculate_grade(-0.1) 应返回 'F'"

	def test_grade_F_underflow_negative_50(self):
		assert self.grade_func(-50.0) == "F", "calculate_grade(-50.0) 应返回 'F'"

	# --- 11. 无效输入类型测试 (原子化) ---

	def test_invalid_type_string_a(self):
		"""测试无效输入：普通字符串"""
		with pytest.raises(TypeError):
			self.grade_func("a")

	def test_invalid_type_string_90(self):
		"""测试无效输入：数字字符串"""
		with pytest.raises(TypeError):
			self.grade_func("90")

	def test_invalid_type_none(self):
		"""测试无效输入：None"""
		with pytest.raises(TypeError):
			self.grade_func(None)

	def test_invalid_type_list(self):
		"""测试无效输入：列表"""
		with pytest.raises(TypeError):
			self.grade_func([90])

	def test_invalid_type_dict(self):
		"""测试无效输入：字典"""
		with pytest.raises(TypeError):
			self.grade_func({"score": 90})
