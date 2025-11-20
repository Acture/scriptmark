from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "email.py"
FUNC_NAME = "is_valid_email"


@pytest.fixture(scope="function")
def student_function(get_function) -> Callable[[str], bool]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)


# ==========================================
# 任务一：电子邮件格式验证测试
# 依据：简化版 RFC 5321
# ==========================================

# (输入字符串, 预期结果)
EMAIL_TEST_CASES = [
	# --- 符合要求的正向案例 (True) ---
	("simple@example.com", True),  # 标准格式
	("user.name@example.com", True),  # Local部分包含点号
	("user_name@example.com", True),  # Local部分包含下划线
	("user-name@example.com", True),  # Local部分包含连字符
	("user+tag@example.com", True),  # Local部分包含加号 (作业明确允许)
	("123456@example.com", True),  # Local部分全是数字
	("user@fudan.edu.cn", True),  # Domain部分包含多级域名 (作业示例)
	("user@sub-domain.com", True),  # Domain部分包含连字符

	# --- 不符合要求的负向案例 (False) ---
	("plainaddress", False),  # 缺少 @ 符号
	("user@localhost", False),  # Domain部分缺少点号 (作业要求：必须包含至少一个点号)
	("@example.com", False),  # 缺少 Local 部分
	("user@", False),  # 缺少 Domain 部分
	("user@.com", False),  # Domain点号位置不当 (域名部分仅允许字母数字连字符)
	("user@com.", False),  # Domain点号在末尾 (通常视为不完整的域名结构)
	("user name@example.com", False),  # 包含空格 (作业未列出空格为允许字符)
	("user@ex ample.com", False),  # Domain包含空格
	("user@example.c#m", False),  # Domain包含非法字符 #
	("user!name@example.com", False),  # Local包含非法字符 ! (作业只允许 _, -, ., +)
]


@pytest.mark.parametrize("email_str, expected", EMAIL_TEST_CASES)
def test_is_valid_email(student_function, email_str, expected):
	"""
	测试 is_valid_email 函数是否符合作业的简化标准。
	标准：
	1. Local: a-z, A-Z, 0-9, _, -, ., +
	2. @: 必须且仅有一个
	3. Domain: 字母, 数字, -, 且必须包含至少一个 .
	"""
	assert student_function(email_str) == expected
