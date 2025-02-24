use crate::run::run_python_code;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::path::Path;
use typed_builder::TypedBuilder;


pub fn generate_twentys(seed: u64) -> Vec<f64> {
	let mut rng = StdRng::seed_from_u64(seed); // ✅ 生成固定随机序列

	(0..20).map(|_| rng.random_range(1.0..=100.0)).collect()
}

pub fn run_lab_one<P: AsRef<Path>>(python_code_path: P, test_inputs: &[f64]) -> Vec<String> {
	let code = std::fs::read_to_string(python_code_path).expect("读取文件失败");

	test_inputs
		.iter()
		.map(|input| {
			run_python_code(
				&code,
				Some(&input.to_string()),
				Some(&[&"math".to_string()]),
			)
		})
		.collect()
}

#[derive(Debug, TypedBuilder, Eq, PartialEq, Hash)]
pub struct Message {
	#[builder(default=String::new())]
	pub title: String,
	#[builder(default=String::new())]
	pub description: String,
}

pub fn judge(
	standard: &[String],
	to_be_judged: &[String],
	judge_function: Option<fn(&str, &str) -> Result<(bool, Vec<Message>), (bool, Vec<Message>)>>,
) -> Vec<(bool, Vec<Message>)> {
	standard
		.iter()
		.zip(to_be_judged.iter())
		.map(|(s, t)| {
			if let Some(judge_function) = judge_function {
				match judge_function(s, t) {
					Ok((is_correct, msg)) => (is_correct, msg),
					Err((is_correct, msg)) => (is_correct, msg),
				}
			} else {
				match s == t {
					true => (true, vec![]),
					false => (
						false,
						vec![Message::builder()
							.title("Value Diff".to_string())
							.description(format!(
								"The standard answer is {}, but the student's answer is {}",
								s, t
							))
							.build()],
					),
				}
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;

	#[test]
	fn test_generate() {
		let seed = 42;
		assert_eq!(generate_twentys(seed), generate_twentys(seed));
	}

	#[test]
	fn test_grade_lab_one_with_file() {
		let default_config = Config::builder().build();

		let lab1_path = default_config.data_dir.join("lab1.py");
		let test_inputs = generate_twentys(42);

		let res = run_lab_one(lab1_path, test_inputs);
		println!("{:?}", res);
	}

	#[test]
	fn test_judge() {
		let default_config = Config::builder().build();

		let lab1_path = default_config.data_dir.join("lab1.py");
		let test_inputs = generate_twentys(42);

		let res = run_lab_one(lab1_path, test_inputs);

		let standard = res.clone();
		let to_be_judged = res.clone();
		judge(standard, to_be_judged, None);
	}

	#[test]
	fn test_run_students_code() {
		let default_config = Config::builder().build();
		let test_class = &crate::class::Class::prepare_class(default_config.data_dir)[0];
		println!("{:?}", test_class)
	}
}
