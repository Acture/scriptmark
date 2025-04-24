score = float(input())

if 90 <= score <= 100:
	print("A")
elif 85 <= score < 90:
	print("A-")
elif 82 <= score < 85:
	print("B+")
elif 78 <= score < 82:
	print("B")
elif 75 <= score < 78:
	print("B-")
elif 71 <= score < 75:
	print("C+")
elif 66 <= score < 71:
	print("C")
elif 62 <= score < 66:
	print("C-")
elif 60 <= score < 62:
	print("D")
elif 0 <= score < 60:
	print("F")
else:
	print("Error!")
