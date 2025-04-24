use std::cmp::max;
use std::collections::HashMap;
use suite::define_test_suite;

type InputType = Vec<(i64, i64)>;
type OutputType = Vec<String>;

fn generate_inputs() -> InputType {
	util::generate(42, 40, 1, 50)
		.chunks_exact(2)
		.map(|chunk| (chunk[0], chunk[1]))
		.collect()
}

fn get_answers() -> OutputType {
	generate_inputs()
		.iter()
		.map(|(a, b)| max(a, b).to_string())
		.collect()
}

fn runner_fn(path: &std::path::Path) -> OutputType {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = std::fs::read_to_string(path).expect("Failed to read file");
	let inputs = generate_inputs();
	inputs
		.iter()
		.map(|(a, b)| {
			match runner::python::run_code::<String>(
				content.clone(),
				Some(format!("{}\n{}\n", a, b)),
				None::<&[String]>,
			) {
				Ok(output) => output,
				Err(e) => format!("Failed to run code: {:?}", e),
			}
		})
		.collect()
}

fn judge_fn(result: &OutputType, expected: &OutputType) -> Vec<suite::test_suite::TestResult> {
	result
		.iter()
		.zip(expected.iter())
		.map(|(result, expected)| {
			let mut res = suite::test_suite::TestResult::builder().build();
			let extracted_res_nums = util::extract_numbers::<i64>(result);
			let extracted_expected_nums = util::extract_numbers::<i64>(expected);
			let res_num_o = extracted_res_nums.last();
			let expected_num_o = extracted_expected_nums.first();
			let (res_num, expected_num) = match (res_num_o, expected_num_o) {
				(Some(res_num), Some(expected_num)) => (res_num, expected_num),
				_ => {
					res.passed = false;
					res.infos.get_or_insert_with(HashMap::new).insert(
						"Failed to parse".to_string(),
						format!("Expected result: <{:?}> Got <{:?}>", result, expected),
					);
					return res;
				}
			};
			if res_num != expected_num {
				res.passed = false;
				res.infos.get_or_insert_with(HashMap::new).insert(
					"Wrong Result".to_string(),
					format!("Expected result: <{:?}> Got <{:?}>", expected_num, res_num, ),
				);
			} else {
				res.passed = true;
			}

			res
		})
		.collect()
}

define_test_suite!(
	pub name = BIGGER_TEST_SUITE,
	inputs = {
		type = InputType,
		init = generate_inputs(),
		clone = |x: & InputType| x.clone()
	},
	answers = {
		type = OutputType,
		init = get_answers(),
		clone = |x: &OutputType| x.clone()
	},
	runner = runner_fn,
	judge = judge_fn
);

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	#[test]
	fn test_generate_inputs() {
		let inputs = generate_inputs();
		println!("Generated inputs: {:?}", inputs);
	}

	#[test]
	fn test_get_answers() {
		let inputs = generate_inputs();
		let answers = get_answers();
		for ((a, b), answer) in inputs.iter().zip(answers.iter()) {
			assert!(*max(a, b) == answer.parse::<i64>().unwrap(),)
		}
	}

	#[test]
	fn test_runner_func() {
		let path = Path::new("../../solutions/bigger.py");
		let res = runner_fn(&path);
		println!("Run results: {:?}", res);
	}

	#[test]
	fn test_judge_func() {
		let path = Path::new("../../solutions/bigger.py");
		let res = runner_fn(&path);
		let answers = get_answers();
		let judge_result = judge_fn(&res, &answers);
		assert!(
			judge_result.iter().all(|x| x.passed),
			"All tests should pass"
		);
	}
}
