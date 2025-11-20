# tests/test_assignment_template.py

import pytest
from typing import Callable, Generator

FILE_NAME = "Lab3_1"
FUNC_NAME_1 = "sort_a"
FUNC_NAME_2 = "get_prime_number_from_a"
FUNC_NAME_3 = "get_number_divisible_by_2or3"
FUNC_NAME_4 = "get_narcissus_number_using_list"
FUNC_NAME_5 = "get_narcissus_number_using_generator"


@pytest.fixture(scope="function")
def student_function_sort_a(get_function) -> Callable[[], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_1, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_get_prime_number_from_a(get_function) -> Callable[[], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_2, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_get_number_divisible_by_2or3(get_function) -> Callable[[], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_3, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_get_narcissus_number_using_list(get_function) -> Callable[[], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_4, FILE_NAME)


@pytest.fixture(scope="function")
def student_function_get_narcissus_number_using_generator(get_function) -> Callable[[], list[int]]:
	"""
	获取学生提交的函数
	"""
	return get_function(FUNC_NAME_5, FILE_NAME)


# --- Tests for each function ---


def test_sort_a_returns_reversed_list(student_function_sort_a):
	"""Tests the sort_a() function by first loading the student's module."""

	result = student_function_sort_a()

	assert isinstance(result, list), "Return value must be a list."
	assert len(result) == 100, "List must contain 100 elements."
	assert result == list(range(99, -1, -1)), "List is not correctly reversed."


def test_get_prime_number_from_a(student_function_get_prime_number_from_a):
	"""Tests the get_prime_number_from_a() function."""
	known_primes = [
		2,
		3,
		5,
		7,
		11,
		13,
		17,
		19,
		23,
		29,
		31,
		37,
		41,
		43,
		47,
		53,
		59,
		61,
		67,
		71,
		73,
		79,
		83,
		89,
		97,
	]
	result = student_function_get_prime_number_from_a()

	assert isinstance(result, list), "Return value must be a list."
	assert sorted(result) == known_primes, "The list of prime numbers is incorrect."


def test_get_number_divisible_by_2or3(student_function_get_number_divisible_by_2or3):
	"""Tests the get_number_divisible_by_2or3() function."""
	expected_result = [n for n in range(100) if n % 2 == 0 or n % 3 == 0]
	result = student_function_get_number_divisible_by_2or3()

	assert isinstance(result, list), "Return value must be a list."
	assert sorted(result) == sorted(
		expected_result
	), "The list of numbers divisible by 2 or 3 is incorrect."


def test_get_narcissus_number_using_list(student_function_get_narcissus_number_using_list):
	"""Tests the list-based narcissus number function."""

	known_narcissus_numbers = [153, 370, 371, 407]
	result = student_function_get_narcissus_number_using_list()

	assert isinstance(result, list), "Return value must be a list."
	assert (
		sorted(result) == known_narcissus_numbers
	), "The list of narcissistic numbers is incorrect."


def test_get_narcissus_number_using_generator(student_function_get_narcissus_number_using_generator):
	"""Tests the generator-based narcissus number function."""
	known_narcissus_numbers = [153, 370, 371, 407]
	result_gen = student_function_get_narcissus_number_using_generator()

	assert isinstance(result_gen, Generator), "Return value must be a generator."
	result_list = list(result_gen)
	assert (
		sorted(result_list) == known_narcissus_numbers
	), "The narcissistic numbers from the generator are incorrect."
