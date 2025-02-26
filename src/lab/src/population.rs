use std::any::Any;
use runner;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use suite;
use suite::test_suite::TestResult;
use util;

static SOLUTION_CODE: &str = include_str!("solutions/population.py");

fn get_answer() -> String {
	runner::python::run_code(SOLUTION_CODE, None, None)
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
			.parse::<i64>()
			.expect("Failed to parse");

		let b_nums = util::extract_numbers::<i64>(b);

		if !b_nums.iter().any(|b_num| a_num == *b_num) {
			res.passed = false;
			res.infos.get_or_insert_with(HashMap::new).insert(
				"Value Diff".to_string(),
				Some(vec![format!(
					"Expected <{}>, Failed to find in <{:?}>",
					a_num, b_nums
				)]),
			);
		}
	}
	res
}

pub fn get_test_suite() -> suite::test_suite::TestSuite {
	let answer = get_answer();
	suite::test_suite::TestSuite::new(
		Box::new(None::<()>),
		Box::new(answer.clone()),
		Box::new(|p: &dyn AsRef<PathBuf>| {
			let path = p.as_ref();
			if !path.exists() {
				panic!("Test file not found: {}", path.display());
			}
			let content = fs::read_to_string(path).expect("Failed to read file");
			Box::new(runner::python::run_code(content, None, None)) as Box<dyn Any>
		}),
		Box::new(|result: &dyn Any, expected: &dyn Any| -> TestResult {
			if let (Some(result_str), Some(expected_str)) = (result.downcast_ref::<String>(), expected.downcast_ref::<String>()) {
				judge_results(expected_str, result_str)
			} else {
				panic!("Failed to downcast")
			}
		}),
	)
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
