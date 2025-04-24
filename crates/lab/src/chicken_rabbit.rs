use itertools::EitherOrBoth;
use itertools::Itertools;
use std::collections::HashMap;
use std::path::Path;
use suite::define_test_suite;
type InputType = Vec<Vec<i64>>;
type OutputType = Vec<Vec<String>>;

const SOLUTION_CODE: &str = include_str!("solutions/chicken_rabbit.py");

fn get_answer(inputs: &InputType) -> OutputType {
	inputs
		.iter()
		.map(|input| {
			let res = runner::python::run_code::<String>(
				SOLUTION_CODE,
				Some(input.iter().map(|x| x.to_string()).join("\n")),
				None::<&[String]>,
			);

			match res {
				Ok(output) => output
					.lines()
					.map(|line| line.to_string())
					.collect::<Vec<_>>(),
				Err(_e) => {
					vec![]
				}
			}
		})
		.collect::<Vec<_>>()
}

fn generate_inputs() -> InputType {
	util::generate(42, 40, 1, 50)
		.chunks_exact(2)
		.map(|chunk|
			// 鸡、兔
			vec![chunk[0] + chunk[1], chunk[0] * 2 + chunk[1] * 4])
		.collect()
}

define_test_suite!(
	pub name = CHICKEN_RABBIT_TEST_SUITE,
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
	let content = std::fs::read_to_string(path).expect("Failed to read file");
	let inputs = generate_inputs();
	inputs
		.iter()
		.map(|input| {
			match runner::python::run_code::<String>(
				content.clone(),
				Some(input.iter().map(|x| x.to_string()).join("\n")),
				None::<&[String]>,
			) {
				Ok(output) => output.lines().map(|line| line.to_string()).collect(),
				Err(e) => vec![format!("Failed to run code: {:?}", e)],
			}
		})
		.collect::<Vec<_>>()
}

fn judge_fn(result: &OutputType, expected: &OutputType) -> Vec<suite::test_suite::TestResult> {
	result
		.iter()
		.zip_longest(expected.iter())
		.map(|pair| {
			let mut res = suite::test_suite::TestResult::builder().build();
			match pair {
				EitherOrBoth::Both(result, expected) => {
					expected.iter().zip(result.iter()).enumerate().for_each(
						|(i, (expected, result))| match i {
							0 => {
								let extracted_expected = util::extract_numbers::<i64>(expected);
								let extracted_result = util::extract_numbers::<i64>(result);
								let first_expected = extracted_expected.first();
								let first_result = extracted_result.first();

								if first_expected.is_none() || first_result.is_none() {
									res.passed = false;
									res.infos.get_or_insert_with(HashMap::new).insert(
										"Missing Rabbit Number".to_string(),
										format!(
											"Expected result: <{:?}>, Got <{:?}>",
											expected, result
										),
									);
									return;
								}

								if first_expected.unwrap() != first_result.unwrap() {
									res.passed = false;
									res.infos.get_or_insert_with(HashMap::new).insert(
										"Wrong Chicken Number".to_string(),
										format!(
											"Expected result: <{:?}>, Got <{:?}>",
											expected, result
										),
									);
								} else {
									res.passed = true;
								}
							}
							1 => {
								let extracted_expected = util::extract_numbers::<i64>(expected);
								let extracted_result = util::extract_numbers::<i64>(result);
								let first_expected = extracted_expected.first();
								let first_result = extracted_result.first();
								if first_expected.is_none() || first_result.is_none() {
									res.passed = false;
									res.infos.get_or_insert_with(HashMap::new).insert(
										"Missing Rabbit Number".to_string(),
										format!(
											"Expected result: <{:?}>, Got <{:?}>",
											expected, result
										),
									);
									return;
								}

								if first_expected.unwrap() != first_result.unwrap() {
									res.passed = false;
									res.infos.get_or_insert_with(HashMap::new).insert(
										"Wrong Rabbit Number".to_string(),
										format!(
											"Expected result: <{:?}>, Got <{:?}>",
											expected, result
										),
									);
								} else {
									res.passed = true;
								}
							}
							_ => {
								res.passed = false;
								res.infos.get_or_insert_with(HashMap::new).insert(
									"Wrong Result".to_string(),
									format!(
										"Expected result: <{:?}>, Got <{:?}>",
										expected, result
									),
								);
							}
						},
					);
				}
				EitherOrBoth::Left(result) => {
					res.passed = false;
					res.infos.get_or_insert_with(HashMap::new).insert(
						"Extra result".to_string(),
						format!("Got extra result: {:?}", result),
					);
				}
				EitherOrBoth::Right(expected) => {
					res.passed = false;
					res.infos.get_or_insert_with(HashMap::new).insert(
						"Missing result".to_string(),
						format!("Expected result: {:?}", expected),
					);
				}
			}
			res
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_inputs() {
		let inputs = generate_inputs();
		assert!(!inputs.is_empty(), "Inputs should not be empty");
		println!("Generated inputs: {:?}", inputs);
	}

	#[test]
	fn test_get_answer() {
		let inputs = generate_inputs();
		let answers = get_answer(&inputs);
		assert_eq!(
			inputs.len(),
			answers.len(),
			"Answers should match the number of inputs"
		);
		for (input, answer) in inputs.iter().zip(answers.iter()) {
			println!("Input: {:?}, Answer: {:?}", input, answer);
		}
	}

	#[test]
	fn test_runner_fn() {
		let inputs = generate_inputs();
		let path = Path::new("../../solutions/chicken_rabbit.py");
		let result = runner_fn(&path);
		assert_eq!(
			inputs.len(),
			result.len(),
			"Result should match the number of inputs"
		);
	}

	#[test]
	fn test_judge_fn() {
		let inputs = generate_inputs();
		let answers = get_answer(&inputs);
		let result = runner_fn(&Path::new("../../solutions/chicken_rabbit.py"));
		let judge_result = judge_fn(&result, &answers);
		assert_eq!(
			inputs.len(),
			judge_result.len(),
			"Judge result should match the number of inputs"
		);
		assert!(
			judge_result.iter().all(|r| r.passed),
			"All tests should pass"
		);
	}
}
