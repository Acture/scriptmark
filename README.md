# ScriptMark: 自动评分工具

[![PyPI](https://img.shields.io/pypi/v/scriptmark)](https://pypi.org/project/scriptmark/)
[![PyPI - Python Version](https://img.shields.io/pypi/pyversions/scriptmark)](https://pypi.org/project/scriptmark/)
[![License](https://img.shields.io/pypi/l/scriptmark)](./LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/Acture/scriptmark/ci.yml)](https://github.com/Acture/scriptmark/actions)

一个可扩展的、用于自动评分学生编程作业的 CLI 工具。

此工具的最初实现是作为 `pytest` 的执行框架，用于自动运行和评估学生提交的 Python 作业。

---

## 核心功能

* **CLI 驱动**: 通过 `typer` 提供清晰、易用的命令行界面。
* **基于 Pytest**: 利用 `pytest` 强大的断言、插件和报告生态系统进行评分。
* **超时控制**: 使用 `pytest-timeout` 严格限制学生代码的执行时间，防止无限循环。
* **精美输出**: 使用 `rich` 在终端中提供格式精美的评分报告。
* **可扩展性**: 旨在未来支持 C++/Java 等其他语言的评分后端（*此功能仍在规划中*）。
* **严格开源**: 采用 **GPLv3** 许可证，确保项目及其衍生品保持开源。

## 安装

您可以通过 `pip` 或 `uv` 从 PyPI 安装 `grader`：

```bash
# 使用 uv (推荐)
uv pip install [您在PyPI上的包名]

# 或使用 pip
pip install [您在PyPI上的包名]

grader --help

```
