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

fn get_test_suite<P: AsRef<PathBuf>>() -> suite::test_suite::TestSuite<Option<()>, String, P> {
	let answer = get_answer();
	suite::test_suite::TestSuite::new(
		None,
		answer.clone(),
		Box::new(|p: &P| {
			if !p.as_ref().exists() {
				panic!("Test file not found: {}", p.as_ref().display());
			}
			let content = fs::read_to_string(&p.as_ref()).expect("Failed to read file");
			runner::python::run_code(content, None, None)
		}),
		Box::new(judge_results),
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
