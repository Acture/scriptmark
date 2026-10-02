def count_words(text):
	# Splits on single spaces only: extra spaces, newlines and "" all miscount.
	return len(text.split(" "))


def title_case(text):
	return text.title()
