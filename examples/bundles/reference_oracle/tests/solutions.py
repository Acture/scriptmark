def answer(value: int) -> int:
	if value < 0:
		raise ValueError("expected a nonnegative value")
	return value * 2


def write_answer(value: int) -> None:
	text: str = str(answer(value))
	print(text)
	with open("answer.txt", "w", encoding="utf-8") as output:
		output.write(text)
