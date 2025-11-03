import pytest

# --- 测试配置 ---
FILE_NAME = "Lab5_2.py"
FUNC_NAME = "calculate_grade"



# --- 1. "A" 级别测试 ---
def test_grade_A_100(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(100.0) == "A", "calculate_grade(100.0) 应返回 'A'"

def test_grade_A_90(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(90.0) == "A", "calculate_grade(90.0) 应返回 'A'"

def test_grade_A_95(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(95.0) == "A", "calculate_grade(95.0) 应返回 'A'"

def test_grade_A_overflow_101(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(101.0) == "A", "calculate_grade(101.0) 应返回 'A'"

def test_grade_A_overflow_200(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(200.0) == "A", "calculate_grade(200.0) 应返回 'A'"

# --- 2. "A-" 级别测试 ---
def test_grade_A_minus_89_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(89.9) == "A-", "calculate_grade(89.9) 应返回 'A-'"

def test_grade_A_minus_85(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(85.0) == "A-", "calculate_grade(85.0) 应返回 'A-'"

def test_grade_A_minus_87_5(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(87.5) == "A-", "calculate_grade(87.5) 应返回 'A-'"

# --- 3. "B+" 级别测试 ---
def test_grade_B_plus_84_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(84.9) == "B+", "calculate_grade(84.9) 应返回 'B+'"

def test_grade_B_plus_82(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(82.0) == "B+", "calculate_grade(82.0) 应返回 'B+'"

# --- 4. "B" 级别测试 ---
def test_grade_B_81_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(81.9) == "B", "calculate_grade(81.9) 应返回 'B'"

def test_grade_B_78(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(78.0) == "B", "calculate_grade(78.0) 应返回 'B'"

# --- 5. "B-" 级别测试 ---
def test_grade_B_minus_77_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(77.9) == "B-", "calculate_grade(77.9) 应返回 'B-'"

def test_grade_B_minus_75(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(75.0) == "B-", "calculate_grade(75.0) 应返回 'B-'"

# --- 6. "C+" 级别测试 ---
def test_grade_C_plus_74_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(74.9) == "C+", "calculate_grade(74.9) 应返回 'C+'"

def test_grade_C_plus_71(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(71.0) == "C+", "calculate_grade(71.0) 应返回 'C+'"

# --- 7. "C" 级别测试 ---
def test_grade_C_70_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(70.9) == "C", "calculate_grade(70.9) 应返回 'C'"

def test_grade_C_66(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(66.0) == "C", "calculate_grade(66.0) 应返回 'C'"

# --- 8. "C-" 级别测试 ---
def test_grade_C_minus_65_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(65.9) == "C-", "calculate_grade(65.9) 应返回 'C-'"

def test_grade_C_minus_62(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(62.0) == "C-", "calculate_grade(62.0) 应返回 'C-'"

# --- 9. "D" 级别测试 ---
def test_grade_D_61_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(61.9) == "D", "calculate_grade(61.9) 应返回 'D'"

def test_grade_D_60(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(60.0) == "D", "calculate_grade(60.0) 应返回 'D'"

# --- 10. "F" 级别测试 ---
def test_grade_F_59_9(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(59.9) == "F", "calculate_grade(59.9) 应返回 'F'"

def test_grade_F_0(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(0.0) == "F", "calculate_grade(0.0) 应返回 'F'"

def test_grade_F_30(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(30.0) == "F", "calculate_grade(30.0) 应返回 'F'"

def test_grade_F_underflow_negative_0_1(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(-0.1) == "F", "calculate_grade(-0.1) 应返回 'F'"

def test_grade_F_underflow_negative_50(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(-50.0) == "F", "calculate_grade(-50.0) 应返回 'F'"

# --- 11. 无效输入类型测试 (原子化) ---

def test_invalid_type_string_a(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	"""测试无效输入：普通字符串"""
	with pytest.raises(TypeError):
		func("a")

def test_invalid_type_string_90(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	"""测试无效输入：数字字符串"""
	with pytest.raises(TypeError):
		func("90")

def test_invalid_type_none(get_function):
	func = get_function(FUNC_NAME, FILE_NAME)
	"""测试无效输入：None"""
	with pytest.raises(TypeError):
		func(None)

def test_invalid_type_list(get_function):
	"""测试无效输入：列表"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func([90])

def test_invalid_type_dict(get_function):
	"""测试无效输入：字典"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func({"score": 90})
