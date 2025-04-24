use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use suite::define_test_suite;
use suite::test_suite::TestResult;

type InputType = Vec<Vec<i64>>;
type OutputType = Vec<Vec<String>>;
fn generate_inputs() -> InputType {
	util::generate(42, 200, 1, 50)
		.chunks_exact(10)
		.map(|chunk| chunk.to_vec())
		.collect()
}

fn get_answers() -> OutputType {
	(1..20).map(|_| vec!["".to_string()]).collect()
}

fn judge_fn(result: &OutputType, _expected: &OutputType) -> Vec<suite::test_suite::TestResult> {
	let mut rec_set = HashSet::new();
	result
		.iter()
		.map(|res| match rec_set.contains(&res) {
			true => TestResult::builder()
				.passed(false)
				.infos(Some(HashMap::from([(
					"Duplicate result".to_string(),
					format!("{:?} from {:?}", res, rec_set),
				)])))
				.build(),
			false => {
				rec_set.insert(res);
				TestResult::builder().passed(true).build()
			}
		})
		.collect::<Vec<TestResult>>()
}

fn runner_fn(path: &Path) -> OutputType {
	if !path.exists() {
		panic!("Test file not found: {}", path.display());
	}
	let content = fs::read_to_string(path).expect("Failed to read file");
	let inputs = generate_inputs();
	inputs
		.iter()
		.map(|input| {
			match runner::python::run_code::<String>(
				content.clone(),
				Some(
                    input
						.iter()
						.map(|x| x.to_string())
						.collect::<Vec<_>>()
						.join(""),
				),
				None::<&[String]>,
			) {
				Ok(output) => output.lines().map(|line| line.to_string()).collect(),
				Err(e) => vec![format!("Failed to run code: {:?}", e)],
			}
		})
		.collect::<Vec<_>>()
}

define_test_suite!(
	pub name = POEM_TEST_SUITE,
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

	#[test]
	fn test_input() {
		let inputs = generate_inputs();
		assert_eq!(inputs.len(), 20);
		for input in inputs.iter() {
			assert_eq!(input.len(), 10);
			for i in input.iter() {
				assert!(*i >= 1 && *i <= 50);
			}
			println!("{:?}", input);
		}
	}
}
