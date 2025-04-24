heads = int(input())
legs = int(input())

# 使用列表推导式计算数量
solutions = [	(chickens, heads - chickens) for chickens in range(heads + 1) if
				2 * chickens + 4 * (heads - chickens) == legs]

if not solutions:
	print(None)
else:
	for solution in solutions:
		chickens, rabbits = solution
		print(chickens)
		print(rabbits)
