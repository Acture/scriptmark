from typing import Callable

import pytest

# --- 测试配置 ---
FILE_NAME = "html.py"
FUNC_NAME = "strip_html"


@pytest.fixture(scope="function")
def student_function(get_function) -> Callable[[str], bool]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME, FILE_NAME)


# ==========================================
# 任务二：HTML 标记清理测试
# 依据：任何以 < 开始、以 > 结束的字符串
# ==========================================

# (包含 HTML 的字符串, 预期纯文本)
HTML_TEST_CASES = [
	# --- 基础标签清理 ---
	("<p>Hello World</p>", "Hello World"),  # 标准成对标签
	("<b>Bold</b>", "Bold"),  # 简单格式标签

	# --- 带属性的标签 ---
	('<a href="index.htm">Link</a>', "Link"),  # 作业示例：带属性的标签
	('<div id="main" class="test">Text</div>', "Text"),

	# --- 单个/自闭合标签 ---
	("Hello<br>World", "HelloWorld"),  # 单个标签，直接移除
	("Image <img src='x.jpg'>", "Image "),  # 图片标签

	# --- 嵌套结构 (简单移除所有 <...>) ---
	("<div><p>Text</p></div>", "Text"),  # 嵌套标签

	# --- 无标签情况 ---
	("Just plain text", "Just plain text"),  # 无 HTML
	("", ""),  # 空字符串

	# --- 多行/复杂情况 ---
	# 假设作业只要求移除标签，不要求处理额外的空白/换行
	("<li>Item 1</li>\n<li>Item 2</li>", "Item 1\nItem 2"),
]


@pytest.mark.parametrize("html_str, expected", HTML_TEST_CASES)
def test_strip_html(student_function, html_str, expected):
	"""
	测试 strip_html 函数。
	标准：移除所有 <...> 格式的子串。
	"""
	result = student_function(html_str)
	assert result == expected
