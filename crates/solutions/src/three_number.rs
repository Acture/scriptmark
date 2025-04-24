use common::define_test_suite;
use common::defines::test_suite::TestResult;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

type InputType = Vec<String>;
type OutputType = Vec<String>;

const SOLUTION_CODE: &str = include_str!("solutions/three_number.py");

fn get_answer(inputs: &InputType) -> OutputType {
	inputs
		.iter()
		.map(|input| {
			let res =
				code_runner::python::run_code::<String>(SOLUTION_CODE, Some(input), None::<&[String]>);
			match res {
				Ok(output) => output,
				Err(e) => panic!("Failed to get answer: {:?}", e),
			}
		})
		.collect::<Vec<_>>()
}

fn generate_inputs() -> Vec<String> {
	common::utils::generate(42, 60, 1, 100)
		.chunks_exact(3)
		.map(|chunk| {
			chunk
				.iter()
				.map(|x| x.to_string())
				.collect::<Vec<String>>()
				.join(",")
		})
		.collect()
}

define_test_suite!(
	pub name = THREE_NUMBER_TEST_SUITE,
	inputs = {
		type = InputType,
		init = generate_inputs(),
		clone = |x: & InputType| x.clone()
	},
	answers = {
		type = OutputType,
		init = get_answer(&INPUTS),
		clone = |x: &OutputType| x.clone()
	},
	runner = runner_fn,
	judge = judge_fn
);

fn runner_fn(path: &Path) -> OutputType {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = fs::read_to_string(path).expect("Failed to read file");
	INPUTS
		.iter()
		.map(|input| {
			match code_runner::python::run_code::<String>(
				content.clone(),
				Some(input),
				None::<&[String]>,
			) {
				Ok(output) => output,
				Err(e) => format!("Failed to run code: {:?}\n\nContent:\n\n{}\n\n", e, content),
			}
		})
		.collect::<Vec<_>>()
}

fn judge_fn(result: &OutputType, expected: &OutputType) -> Vec<common::defines::test_suite::TestResult> {
	result
		.iter()
		.zip(expected.iter())
		.map(|(res, ans)| {
			let mut test_result = TestResult::builder().build();
			res.lines()
				.zip(ans.lines())
				.enumerate()
				.for_each(|(i, (res_line, ans_line))| {
					let res_nums = common::utils::extract_numbers::<i64>(res_line);
					let ans_nums = common::utils::extract_numbers::<i64>(ans_line);

					let t = match i {
						0 => "Maximum",
						1 => "Minimum",
						2 => "Median",
						_ => "Unknown",
					};

					if res_nums.len() != ans_nums.len() {
						test_result
							.infos
							.get_or_insert_with(HashMap::new)
							.entry("More Than One Number".to_string())
							.or_insert(format!(
								"Expected: <{}>, Got: <{}>",
								res_nums.len(),
								ans_nums.len()
							));
					}
					let res_num = match res_nums.first() {
						Some(value) => *value,
						None => {
							test_result.passed = false;
							test_result
								.infos
								.get_or_insert_with(HashMap::new)
								.entry(format!("{} Result", t))
								.or_insert(format!("Failed to extract number: {:?}", res));
							return;
						}
					};
					let ans_num = match ans_nums.first() {
						Some(value) => *value,
						None => {
							test_result.passed = false;
							test_result
								.infos
								.get_or_insert_with(HashMap::new)
								.entry(format!("{} Answer", t))
								.or_insert(format!("Failed to extract number: {:?}", ans));
							return;
						}
					};

					if res_num != ans_num {
						test_result.passed = false;
						test_result
							.infos
							.get_or_insert_with(HashMap::new)
							.entry("Value Diff".to_string())
							.or_insert(format!("Expected <{}>, Got <{}>", res_num, ans_num));
					} else {
						test_result.passed = true;
					}
				});

			test_result
		})
		.collect::<Vec<TestResult>>()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_inputs() {
		let inputs = generate_inputs();
		assert!(!inputs.is_empty(), "Inputs should not be empty");
	}

	#[test]
	fn test_answers() {
		let inputs = generate_inputs();
		let answers = get_answer(&inputs);
		for answer in answers.iter() {
			assert_eq!(answer.lines().count(), 3)
		}
	}

	#[test]
	fn test_runner() {
		let inputs = generate_inputs();
		let answers = get_answer(&inputs);
		for (input, answer) in inputs.iter().zip(answers.iter()) {
			println!("Input: {:?}, Answer: {:?}", input, answer);
		}
	}

	#[test]
	fn test_judge() {
		let inputs = generate_inputs();
		let answers = get_answer(&inputs);
		let results = judge_fn(&answers, &answers);
		for result in results.iter() {
			assert!(result.passed, "Test failed: {:?}", result);
		}
	}
}
