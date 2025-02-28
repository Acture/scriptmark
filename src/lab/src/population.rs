use runner;
use std::any::Any;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite;
use suite::test_suite::TestResult;
use util;

static SOLUTION_CODE: &str = include_str!("solutions/population.py");

fn get_answer() -> Vec<String> {
	vec![runner::python::run_code(SOLUTION_CODE, None, None)]
}

fn judge_results<'a, 'b>(answer: &'a String, to_test: &'b String) -> TestResult {
	let lines = answer
		.split("\n")
		.filter(|line| !line.trim().is_empty())
		.into_iter()
		.zip(
            to_test
				.split("\n")
				.filter(|line| !line.trim().is_empty())
				.into_iter(),
		)
		.into_iter();

	let mut res = TestResult::builder().passed(true).build();

	for (a, b) in lines {
		let a_num = a
			.split(" ")
			.last()
			.expect("Failed to get last element")
			.parse::<f64>()
			.expect("Failed to parse");

		let b_nums = util::extract_numbers::<f64>(b);
		if b_nums.len() > 2 {
			res.passed = false;
			res.infos
				.get_or_insert_with(HashMap::new)
				.entry("More Than Two Number".to_string())
				.or_insert(format!("Expected: <{}>, Got: <{}>", answer, to_test));
			continue;
		}

		if b_nums.len() == 0 {
			res.passed = false;
			res.infos
				.get_or_insert_with(HashMap::new)
				.entry("No Number".to_string())
				.or_insert(format!("Expected: <{}>, Got: <{}>", answer, to_test));
		}

		let b_num = match b_nums.len() {
			0 => continue,
			1 => b_nums[0],
			2 => b_nums[1],
			_ => panic!("Invalid number count"),
		};

		if (a_num - b_num).abs() > 0.0001 * a_num {
			res.passed = false;
			res.infos
				.get_or_insert_with(HashMap::new)
				.entry("Value Diff".to_string())
				.or_insert(format!("Expected <{}>, Got <{}>", a_num, b_num));
		}
	}
	res
}

pub fn get_test_suite() -> Box<dyn suite::test_suite::TestSuiteTrait> {
	let answer = get_answer();
	let input = None::<()>;
	let runner = |path: &Path| {
		if !path.exists() {
			panic!("Test file not found: {}", path.display());
		}
		let content = fs::read_to_string(path).expect("Failed to read file");
		vec![runner::python::run_code(content, None, None)]
	};
	let judge = |result: &[String], expected: &[String]| -> Vec<TestResult> {
		let result_one = result.first().expect("Failed to get result");
		let expected_one = expected.first().expect("Failed to get expected");
		if result_one.lines().count() != expected_one.lines().count() {
			return vec![TestResult::builder()
				.passed(false)
				.infos(Some(HashMap::from_iter(vec![(
					String::from("Line Count"),
					format!("Expected: <{}>, Got: <{}>", expected_one, result_one),
				)])))
				.build()]; // You might be missing the .build() call here
		}
		result_one
			.lines()
			.zip(expected_one.lines())
			.map(|(result, expected)| judge_results(&expected.to_string(), &result.to_string()))
			.collect()
	};

	Box::new(suite::test_suite::TestSuite::new(
        input, answer, runner, judge,
	))
}
mod test {
	use super::*;
	#[test]
	fn test() {
		println!("{}", SOLUTION_CODE);
		assert_eq!(1, 1);
	}

	#[test]
	fn test_judge() {
		let answer = get_answer();
		let res = vec![answer.clone()]
			.iter()
			.chain(
                vec![
					String::from("1"),
					String::from("2"),
					String::from("3"),
					String::from("4"),
					String::from("5 2321"),
				]
				.iter(),
			)
			.map(|s| judge_results(&answer, s))
			.collect::<Vec<_>>();
		println!("{:?}", res);
	}
}
