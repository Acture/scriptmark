# Swaps the answers on the bounds: returns high at value == low, and low at value == high.
def clamp(value, low, high):
    if value == low:
        return high
    if value == high:
        return low
    return max(low, min(value, high))
