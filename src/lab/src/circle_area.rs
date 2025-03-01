use runner;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite::define_test_suite;
use suite::test_suite::TestResult;

const SOLUTION_CODE: &str = include_str!("solutions/circle_area.py");

pub fn get_answer(input: f64) -> String {
	match runner::python::run_code::<String>(
		SOLUTION_CODE,
		Some(&input.to_string()),
		None::<&[String]>,
	) {
		Ok(output) => output,
		Err(e) => panic!("Failed to get answer: {:?}", e),
	}
}

// lazy_static! {
// 	static ref INPUTS: Vec<f64> = util::generate(42, 20, 1.0, 100.0);
// 	static ref ANSWERS: Vec<String> = INPUTS
// 		.iter()
// 		.map(|input| get_answer(*input))
// 		.collect::<Vec<_>>();
// 	pub static ref CIRCLE_AREA_TEST_SUITE: TestSuite<
// 		Vec<f64>,
// 		Vec<String>,
// 		for<'a> fn(&'a Path) -> Vec<String>,
// 		for<'a, 'b> fn(&'a Vec<String>, &'b Vec<String>) -> Vec<suite::test_suite::TestResult>,
// 	> = TestSuite::builder()
// 		.inputs(INPUTS.to_vec())
// 		.answers(ANSWERS.to_vec())
// 		.runner(RUNNER_FUNC as fn(&Path) -> Vec<String>)
// 		.judge(JUDGE_FUNC as fn(&Vec<String>, &Vec<String>) -> Vec<TestResult>)
// 		.build();
// }

define_test_suite!(
	pub name = CIRCLE_AREA_TEST_SUITE,
	inputs = {
		type = Vec<f64>,
		init = util::generate(42, 20, 1.0, 100.0),
		clone = |x: &Vec<f64>| x.to_vec()
	},
	answers = {
		type = Vec<String>,
		init = INPUTS.iter().map(|input| get_answer(*input)).collect::<Vec<_>>(),
		clone = |x: &Vec<String>| x.to_vec()
	},
	runner = runner_fn,
	judge = judge_fn
);

fn runner_fn(path: &Path) -> Vec<String> {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = fs::read_to_string(&path).expect("Failed to read file");
	INPUTS
		.iter()
		.map(|input| {
			match runner::python::run_code(
				content.clone(),
				Some(input.to_string()),
				None::<&[String]>,
			) {
				Ok(output) => output,
				Err(e) => format!("Failed to run code: {:?}\n\nContent:\n\n{}\n\n", e, content),
			}
		})
		.collect::<Vec<_>>()
}

fn judge_fn(result: &Vec<String>, expected: &Vec<String>) -> Vec<TestResult> {
	result
		.iter()
		.zip(expected.into_iter())
		.map(|(result, expected)| judge(result, expected))
		.collect::<Vec<TestResult>>()
}
fn judge(s: &str, t: &str) -> TestResult {
	let mut res = TestResult::builder().build();
	s.lines()
		.zip(t.lines())
		.enumerate()
		.for_each(|(i, (s_line, t_line))| match i {
			0 => {
				let s_extracted = util::extract_numbers::<f64>(s_line);
				let t_extracted = util::extract_numbers::<f64>(t_line);

				if s_extracted.len() != 1 || t_extracted.len() != 1 {
					res.infos
						.get_or_insert_with(HashMap::new)
						.entry("More Than One Number".to_string())
						.or_insert(format!(
							"Expected: 1, Got: s: {}, t: {}",
							s_extracted.len(),
							t_extracted.len()
						));
				}
				let s_area = match s_extracted.first() {
					Some(value) => *value,
					None => {
						res.passed = false;
						res.infos
							.get_or_insert_with(HashMap::new)
							.entry("Area".to_string())
							.or_insert(format!(
								"Failed to extract number from line {}: {:?}",
								i, s
							));
						return;
					}
				};
				let t_area = match t_extracted.first() {
					Some(value) => *value,
					None => {
						res.passed = false;
						res.infos
							.get_or_insert_with(HashMap::new)
							.entry("Area".to_string())
							.or_insert(format!(
								"Failed to extract number from line {}: {:?}",
								i, t
							));
						return;
					}
				};
				let offset_percent = 0.0001;
				if (s_area - t_area).abs() > offset_percent * s_area {
					res.passed = false;
					res.infos
						.get_or_insert_with(HashMap::new)
						.entry("Area".to_string())
						.or_insert(format!("Expected: <{}>, Got: <{}>", t_area, s_area));
				} else {
					res.passed = true;
				}
			}
			1 => {
				let s_count = match util::extract_numbers::<f64>(t_line).pop() {
					Some(value) => value,
					None => {
						res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
						res.additional_infos
							.get_or_insert_with(HashMap::new)
							.entry("Count".to_string())
							.or_insert(format!(
								"Failed to extract number from line {}: {:?}",
								i, s
							));
						return;
					}
				};
				let t_count = match util::extract_numbers::<f64>(t_line).pop() {
					Some(value) => value,
					None => {
						res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
						res.additional_infos
							.get_or_insert_with(HashMap::new)
							.entry("Count".to_string())
							.or_insert(format!(
								"Failed to extract number from line {}: {:?}",
								i, t
							));
						return;
					}
				};
				if s_count != t_count {
					res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
					res.additional_infos
						.get_or_insert_with(HashMap::new)
						.entry("Count".to_string())
						.or_insert(format!("Expected: <{}>, Got: <{}>", t_count, s_count));
				} else {
					res.additional_status = Some(suite::test_suite::AdditionalStatus::Full);
				}
			}
			_ => {
				res.passed = false;
				res.infos
					.get_or_insert_with(HashMap::new)
					.entry("Extra Lines".to_string())
					.or_insert(format!("Extra line: {}", s_line));
			}
		});
	res
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_get_answer() {
		let input = 3.14;
		let answer = get_answer(input);
		println!("{}", answer);

		assert_eq!(
			answer,
			"Radius？ Area is:  30.974846927333928\nIts integral part is a 2-digit number.\n"
		);
	}
}
