import typer
from . import run_test, summary  # 从同级目录导入 run 和 summarize 模块

# 创建一个 Typer 应用实例
app = typer.Typer(
	name="grader",
	help="一个用于 Python 作业的自动化评测和报告工具。",
	no_args_is_help=True
)

app.command(name="run")(run_test.run_tests)

app.command(name="summarize")(summary.generate_summary)

if __name__ == "__main__":
	app()
