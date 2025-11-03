import pytest

# --- 测试配置 ---
FILE_NAME = "Lab5_1.py"
FUNC_NAME = "find_larger_number"




# --- 1. 有效输入测试 (原子化) ---

def test_num1_less_than_num2_positive(get_function):
	"""测试: 3 < 5"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(3, 5) == 5, "find_larger_number(3, 5) 应返回 5"

def test_num1_less_than_num2_negative(get_function):
	"""测试: -3 < -2"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(-3, -2) == -2, "find_larger_number(-3, -2) 应返回 -2"

def test_num1_less_than_num2_mixed(get_function):
	"""测试: -1 < 1"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(-1, 1) == 1, "find_larger_number(-1, 1) 应返回 1"

def test_num1_greater_than_num2_float(get_function):
	"""测试: 2.5 > 2.4"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(2.5, 2.4) == 2.5, "find_larger_number(2.5, 2.4) 应返回 2.5"

def test_num1_greater_than_num2_mixed(get_function):
	"""测试: 10 > -10"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(10, -10) == 10, "find_larger_number(10, -10) 应返回 10"

def test_equal_zero(get_function):
	"""测试: 0 == 0"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(0, 0) == 0, "find_larger_number(0, 0) 应返回 0"

def test_equal_positive_float(get_function):
	"""测试: 7.7 == 7.7"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(7.7, 7.7) == 7.7, "find_larger_number(7.7, 7.7) 应返回 7.7"

def test_equal_negative_int(get_function):
	"""测试: -5 == -5"""
	func = get_function(FUNC_NAME, FILE_NAME)
	assert func(-5, -5) == -5, "find_larger_number(-5, -5) 应返回 -5"

def test_equal_check_return_type(get_function):
	"""测试: 5 == 5，并检查返回类型"""
	# 作业要求返回 float，但如果实现是 max(num1, num2)，输入两个 int 就会返回 int
	# 我们可以放宽到 int 或 float
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(5, 5)
	assert result == 5
	assert isinstance(result, (int, float)), f"返回值类型应为 float 或 int，但实际为 {type(result)}"

# --- 2. 无效输入类型测试 (原子化) ---

def test_invalid_type_str_vs_int(get_function):
	"""测试无效输入：'a', 1"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func("a", 1)

def test_invalid_type_int_vs_str(get_function):
	"""测试无效输入：1, 'b'"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func(1, "b")

def test_invalid_type_str_vs_str(get_function):
	"""测试无效输入：'a', 'b'"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func("a", "b")

def test_invalid_type_none_vs_int(get_function):
	"""测试无效输入：None, 1"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func(None, 1)

def test_invalid_type_int_vs_none(get_function):
	"""测试无效输入：1, None"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func(1, None)

def test_invalid_type_none_vs_none(get_function):
	"""测试无效输入：None, None"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func(None, None)

def test_invalid_type_list_vs_int(get_function):
	"""测试无效输入：[1, 2], 3"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func([1, 2], 3)

def test_invalid_type_int_vs_dict(get_function):
	"""测试无效输入：4, {'a': 1}"""
	func = get_function(FUNC_NAME, FILE_NAME)
	with pytest.raises(TypeError):
		func(4, {"a": 1})
