def mean(nums):
    if not nums:
        return 0
    return sum(nums) // len(nums)


def clamp(x, lo, hi):
    return max(lo, min(x, hi))
