
# --- 测试配置 ---
FILE_NAME = "k.py"
FUNC_NAME = "find_k_numbers"


def test_k_numbers_count(get_function):
	"""
	测试 100000 以内的 K 数数量是否为 6
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	assert len(result) == 6, f"K 数的数量应为 6，但找到了 {len(result)} 个"


def test_k_number_1_exists(get_function):
	"""测试 K 数 1 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)

	assert 1 in result, "K 数 1 未在结果中找到"


def test_k_number_81_exists(get_function):
	"""测试 K 数 81 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)

	assert 81 in result, "K 数 81 未在结果中找到"


def test_k_number_100_exists(get_function):
	"""测试 K 数 100 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	assert 100 in result, "K 数 100 未在结果中找到"


def test_k_number_2025_exists(get_function):
	"""测试 K 数 2025 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	assert 2025 in result, "K 数 2025 未在结果中找到"


def test_k_number_3025_exists(get_function):
	"""测试 K 数 3025 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	assert 3025 in result, "K 数 3025 未在结果中找到"


def test_k_number_9801_exists(get_function):
	"""测试 K 数 9801 是否存在"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	assert 9801 in result, "K 数 9801 未在结果中找到"


def test_k_numbers_are_all_correct(get_function):
	"""
	测试所有 K 数是否完全匹配 (忽略顺序)
	"""
	func = get_function(FUNC_NAME, FILE_NAME)
	result = func(100000)
	expected_k_numbers = [1, 81, 100, 2025, 3025, 9801]
	assert sorted(result) == sorted(expected_k_numbers), \
		f"找到的K数不正确。预期 {expected_k_numbers}，但得到 {sorted(result)}"
