# tests/test_assignment_template.py

import pytest
from typing import Generator


# --- Helper function to reduce repetition ---
def get_student_module(student_submission):
	"""
	A helper to get the main module from the student's submission.

	This uses the get_module method from our Submission manager. For this assignment,
	we assume the student submits exactly one '.py' file. The get_module method
	will correctly fail if zero or more than one .py file is found.
	"""
	return student_submission.get_module(ends_with="Lab3_1.py")


def get_function(module, func_name):
	"""Safety checks if a function exists before testing it."""
	if not hasattr(module, func_name):
		pytest.skip(f"Function '{func_name}' not found in the submission.")
	return getattr(module, func_name)


# --- Tests for each function ---

def test_sort_a_returns_reversed_list(student_submission):
	"""Tests the sort_a() function by first loading the student's module."""
	module = get_student_module(student_submission)
	sort_a_func = get_function(module, 'sort_a')

	result = sort_a_func()

	assert isinstance(result, list), "Return value must be a list."
	assert len(result) == 100, "List must contain 100 elements."
	assert result == list(range(99, -1, -1)), "List is not correctly reversed."


def test_get_prime_number_from_a(student_submission):
	"""Tests the get_prime_number_from_a() function."""
	module = get_student_module(student_submission)
	get_primes_func = get_function(module, 'get_prime_number_from_a')

	known_primes = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97]
	result = get_primes_func()

	assert isinstance(result, list), "Return value must be a list."
	assert sorted(result) == known_primes, "The list of prime numbers is incorrect."


def test_get_number_divisible_by_2or3(student_submission):
	"""Tests the get_number_divisible_by_2or3() function."""
	module = get_student_module(student_submission)
	get_divisible_func = get_function(module, 'get_number_divisible_by_2or3')

	expected_result = [n for n in range(100) if n % 2 == 0 or n % 3 == 0]
	result = get_divisible_func()

	assert isinstance(result, list), "Return value must be a list."
	assert sorted(result) == sorted(expected_result), "The list of numbers divisible by 2 or 3 is incorrect."


def test_get_narcissus_number_using_list(student_submission):
	"""Tests the list-based narcissus number function."""
	module = get_student_module(student_submission)
	get_narcissus_list_func = get_function(module, 'get_narcissus_number_using_list')

	known_narcissus_numbers = [153, 370, 371, 407]
	result = get_narcissus_list_func()

	assert isinstance(result, list), "Return value must be a list."
	assert sorted(result) == known_narcissus_numbers, "The list of narcissistic numbers is incorrect."


def test_get_narcissus_number_using_generator(student_submission):
	"""Tests the generator-based narcissus number function."""
	module = get_student_module(student_submission)
	get_narcissus_gen_func = get_function(module, 'get_narcissus_number_using_generator')

	known_narcissus_numbers = [153, 370, 371, 407]
	result_gen = get_narcissus_gen_func()

	assert isinstance(result_gen, Generator), "Return value must be a generator."
	result_list = list(result_gen)
	assert sorted(result_list) == known_narcissus_numbers, "The narcissistic numbers from the generator are incorrect."
