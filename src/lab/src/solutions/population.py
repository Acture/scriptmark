#  每7秒1人出生
# ´每13秒1人死亡
# ´每45秒1人移入
# ´每79秒1人移出

current_pop = 3120324986
secs_per_year = 365 * 24 * 60 * 60

for i in range(5):
	# birth
	current_pop += secs_per_year // 7
	# death
	current_pop -= secs_per_year // 13
	# imigration
	current_pop += secs_per_year // 45
	# emigration
	current_pop -= secs_per_year // 79

	print(current_pop)
