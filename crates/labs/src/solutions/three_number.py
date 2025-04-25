nums = input().split(",")
nums = [int(num) for num in nums]
nums.sort()
max_num = nums[2]
median_num = nums[1]
min_num = nums[0]
print(max_num)
print(min_num)
print(median_num)
