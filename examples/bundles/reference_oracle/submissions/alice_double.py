def double(value: int) -> int:
	if value < 0:
		raise ValueError("negative")
	return value + value


def write_double(value: int) -> None:
	text: str = str(double(value))
	print(text)
	with open("answer.txt", "w", encoding="utf-8") as output:
		output.write(text)
