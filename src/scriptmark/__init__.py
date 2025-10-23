import logging

from rich.logging import RichHandler

logging.basicConfig(
	level="INFO",
	format="%(message)s",  # 格式由 RichHandler 控制，这里只需传递消息
	datefmt="[%X]",  # 时间格式
	handlers=[RichHandler(rich_tracebacks=True)],  # 使用 RichHandler
)
