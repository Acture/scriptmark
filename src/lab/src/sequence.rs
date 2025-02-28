use runner;
use std::any::Any;
use suite;
use suite::test_suite::TestResult;
use util;

static SOLUTION_CODE: &str = include_str!("solutions/sequence.py");

fn get_answer() -> Vec<String> {
	let (output, trace) = runner::python::run_code_with_trace(SOLUTION_CODE, None, None);
	vec![trace]
}

fn judge_results<'a, 'b>(answer: &'a String, to_test: &'b String) -> TestResult {}

pub fn get_test_suite() -> Box<dyn suite::test_suite::TestSuiteTrait> {}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_get_answer() {
		let answer = get_answer();
		println!("{:?}", answer);
	}
}
