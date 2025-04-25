import math

r = input("Radius？ ")
r = float(r)
area = math.pi * r * r
print("Area is: ", area)
integral_part_count = len(str(int(area)))
print(f"Its integral part is a {integral_part_count}-digit number.")
