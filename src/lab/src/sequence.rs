use runner;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite::define_test_suite;

const SOLUTION_CODE: &str = include_str!("solutions/sequence.py");

type InputType = Option<()>;
type OutputType = Vec<HashMap<String, Vec<i64>>>;

fn get_answer() -> OutputType {
	let result = runner::python::run_code_with_trace::<String, Vec<i64>>(SOLUTION_CODE, None::<&String>, None::<&[String]>);

	match result {
		Ok((_output, trace)) => {
			let mut t = trace.into_iter().collect::<Vec<_>>();
			t.sort_by_key(|x| x.0);
			t.into_iter()
				.filter_map(|(_, output)| {
					if output.is_empty() {
						None
					} else {
						Some(output)
					}
				})
				.collect::<Vec<_>>()
		}

		Err(e) => panic!("Failed to get answer: {:?}", e),
	}
}

fn runner_fn(path: &Path) -> OutputType {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = fs::read_to_string(path).expect("Failed to read file");
	let res = runner::python::run_code_with_trace::<String, Vec<i64>>(content, None::<&String>, None::<&[String]>);
	match res {
		Ok((_output, trace)) => {
			let mut t = trace.into_iter().collect::<Vec<_>>();
			t.sort_by_key(|x| x.0);
			t.into_iter()
				.filter_map(|(_, output)| {
					if output.is_empty() {
						None
					} else {
						Some(output)
					}
				})
				.collect::<Vec<_>>()
		}
		Err(e) => panic!("Failed to run code: {:?}", e),
	}
}

fn judge_fn(_result: &OutputType, _answer: &OutputType) -> Vec<suite::test_suite::TestResult> {
	_result
		.iter()
		.zip(_answer.iter())
		.map(|(result, answer)| {
			if result.len() != answer.len() {
				return suite::test_suite::TestResult::builder()
					.passed(false)
					.infos(Some(HashMap::from_iter(vec![(
						String::from("Length"),
						format!("Expected: <{:?}>, Got: <{:?}>", answer, result),
					)])))
					.build();
			}
			for (r, a) in result.iter().zip(answer.iter()) {
				if r != a {
					return suite::test_suite::TestResult::builder()
						.passed(false)
						.infos(Some(HashMap::from_iter(vec![(
							String::from("Output"),
							format!("Expected: <{:?}>, Got: <{:?}>", a, r),
						)])))
						.build();
				}
			}
			suite::test_suite::TestResult::builder()
				.passed(true)
				.build()
		})
		.collect()
}

define_test_suite!(
	pub name = SEQUENCE_TEST_SUITE,
	inputs = {
		type = InputType,
		init = None::<()>,
		clone = |x: & InputType| *x
	},
	answers = {
		type = OutputType,
		init = get_answer(),
		clone = |x: &OutputType| x.clone()
	},
	runner = runner_fn,
	judge = judge_fn
);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_get_answer() {
		let answer = get_answer();
		for a in &answer {
			println!("{:?}", a);
		}
		assert_eq!(answer.len(), 3);
	}
}
