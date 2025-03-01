use runner;

const SOLUTION_CODE: &str = include_str!("solutions/sequence.py");

fn get_answer() -> Vec<String> {
	let (_output, trace) = runner::python::run_code_with_trace(SOLUTION_CODE, None, None);
	let mut res = trace.iter().collect::<Vec<_>>();
	res.sort_by_key(|(k, _v)| *k);
	res.into_iter().filter_map(|(_k, v_map)| {
		if v_map.len() > 1 {
			let mut v_vec = v_map.iter().collect::<Vec<_>>();
			v_vec.sort_by_key(|(k, _v)| *k);
			v_vec.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join(" ").into()
		} else {
			None
		}
	})
		.collect()
}

// lazy_static!(
// 	static ref ANSWERS: Vec<String> = get_answer();
// 	pub static ref SEQUENCE_TEST_SUITE: suite::test_suite::TestSuite<
// 		Vec<String>,
// 		Vec<String>,
// 		for<'a> fn(&'a std::path::Path) -> Vec<String>,
// 		for<'a, 'b> fn(&'a Vec<String>, &'b Vec<String>) -> Vec<suite::test_suite::TestResult>,
// 	> = suite::test_suite::TestSuite::builder()
// 		.inputs(Vec::new())
// 		.answers(ANSWERS.to_vec())
// 		.runner(RUNNER_FUNC as fn(&std::path::Path) -> Vec<String>)
// 		.judge(JUDGE_FUNC as fn(&Vec<String>, &Vec<String>) -> Vec<suite::test_suite::TestResult>)
// 		.build();
// )


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_get_answer() {
		let answer = get_answer();
		assert_eq!(answer.len(), 2);
	}
}
