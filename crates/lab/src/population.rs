use std::clone::Clone;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite::define_test_suite;
use suite::test_suite::TestResult;

const SOLUTION_CODE: &str = include_str!("solutions/population.py");

fn judge_results(answer: & String, to_test: & String) -> TestResult {
	let lines = answer
		.split("\n")
		.filter(|line| !line.trim().is_empty())
		.zip(
			to_test
				.split("\n")
				.filter(|line| !line.trim().is_empty())
		);

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

		if b_nums.is_empty(){
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
//
// lazy_static! {
// 	static ref INPUTS: Option<()> = None::<()>;
// 	static ref ANSWERS: String = runner::python::run_code(SOLUTION_CODE, None, None);
// 	pub static ref POPULATION_TEST_SUITE: TestSuite<
// 	Option<()>,
// 	String,
// 	for<'a> fn(&'a Path) -> String,
// 	for<'a, 'b> fn(&'a String, &'b String) -> Vec<suite::test_suite::TestResult>,
// > = TestSuite::builder()
// 	.inputs(INPUTS.clone())
// 	.answers(ANSWERS.clone())
// 	.runner(RUNNER_FUNC as fn(&Path) -> String)
// 	.judge(JUDGE_FUNC as fn(&String, &String) -> Vec<TestResult>)
// 	.build();
// }

define_test_suite!(
	pub name = POPULATION_TEST_SUITE,
	inputs = {
		type = Option<()>,
		init = None::<()>,
		clone = |x: & Option<()>| *x
	},
	answers = {
		type = String,
		init = match runner::python::run_code(SOLUTION_CODE, None::<&String>, None::<&[String]>){
			Ok(output) => output,
			Err(e) => panic!("Failed to get answer: {:?}", e),
		},
		clone = |x: &String| x.clone()
	},
	runner = runner_fn,
	judge = judge_fn
);

fn runner_fn(path: &Path) -> String {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = fs::read_to_string(path).expect("Failed to read file");

	match runner::python::run_code(&content, None::<String>, None::<&[String]>) {
		Ok(output) => output,
		Err(e) => format!("Failed to run code: {:?}\n\nContent:\n\n{}\n\n", e, content),
	}
}

fn judge_fn(result: &String, expected: &String) -> Vec<TestResult> {
	if result.lines().count() != expected.lines().count() {
		return vec![TestResult::builder()
			.passed(false)
			.infos(Some(HashMap::from_iter(vec![(
				String::from("Line Count"),
				format!("Expected: <{}>, Got: <{}>", expected, result),
			)])))
			.build()]; // You might be missing the .build() call here
	}
	result
		.lines()
		.zip(expected.lines())
		.map(|(result, expected)| judge_results(&expected.to_string(), &result.to_string()))
		.collect()
}
