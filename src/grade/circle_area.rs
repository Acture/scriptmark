use crate::run;
use crate::run::run_python_code;
use std::path::Path;

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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;
	use crate::run::{generate, judge};

	#[test]
	fn test_generate() {
		let seed = 42;
		assert_eq!(generate(seed), generate(seed));
	}

	#[test]
	fn test_grade_lab_one_with_file() {
		let default_config = Config::builder().build();

		let lab1_path = default_config.data_dir.join("lab1.py");
		let test_inputs = generate(42);

		let res = run_lab_one(lab1_path, test_inputs);
		println!("{:?}", res);
	}

	#[test]
	fn test_judge() {
		let default_config = Config::builder().build();

		let lab1_path = default_config.data_dir.join("lab1.py");
		let test_inputs = generate(42);

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

pub fn judge_func(
	s: &str,
	t: &str,
) -> Result<(bool, Vec<run::Message>), (bool, Vec<run::Message>)> {
	let s_lines: Vec<_> = s
		.split("\n")
		.filter(|line| !line.trim().is_empty())
		.collect();
	let t_lines: Vec<_> = t
		.split("\n")
		.filter(|line| !line.trim().is_empty())
		.collect();
	let required_s: f64 = s_lines
		.get(0)
		.ok_or_else(|| {
			(
				false,
				vec![run::Message::builder()
					.title("Standard Line 0 Error".to_string())
					.build()],
			)
		})?
		.split(':')
		.last()
		.and_then(|s| s.trim().parse::<f64>().ok())
		.ok_or_else(|| {
			(
				false,
				vec![run::Message::builder()
					.title("Standard Parse Error".to_string())
					.description(format!("The standard value is {}", s))
					.build()],
			)
		})?;
	let required_t: f64 = t_lines
		.get(0)
		.ok_or_else(|| {
			(
				false,
				vec![run::Message::builder()
					.title("Tested Line 0 Error".to_string())
					.build()],
			)
		})?
		.split(':')
		.last()
		.and_then(|s| s.trim().parse::<f64>().ok())
		.ok_or_else(|| {
			(
				false,
				vec![run::Message::builder()
					.title("Tested Parse Error".to_string())
					.description(format!(
						"The tested value is {}. Expected {}",
						t, required_s
					))
					.build()],
			)
		})?;

	let passed;
	let mut messages = Vec::new();

	if required_s != required_t {
		let diff = ((required_s - required_t) / required_s).abs() * 100.0;
		if diff < 1.0 {
			let mut message = run::Message::builder()
				.title("Small Value Difference".to_string())
				.build();
			message
				.description
				.push_str(format!("The difference is {} %", diff).as_str());
			passed = true;
			messages.push(message);
		} else {
			passed = false;
			let mut message = run::Message::builder()
				.title("Value Difference".to_string())
				.build();
			message
				.description
				.push_str(format!("The difference is {} %", diff).as_str());
			messages.push(message);
		}
	} else {
		passed = true;
	}

	let additional_s = s_lines.get(1..).unwrap_or(&[]);
	let additional_t = t_lines.get(1..).unwrap_or(&[]);

	let mut additional_s = additional_s.iter();
	let mut additional_t = additional_t.iter();

	if additional_t.len() > 0 {
		messages.push(
            run::Message::builder()
				.title("完成选做".to_string())
				.build(),
		);
		loop {
			let s = additional_s.next();
			let t = additional_t.next();
			match (s, t) {
				(Some(s), Some(t)) => {
					if s != t {
						messages.push(
                            run::Message::builder()
								.title("Additional Output Difference".to_string())
								.description(format!("Desired <{}>, Given <{}>", s, t))
								.build(),
						);
					}
				}
				(Some(s), None) => {
					messages.push(
                        run::Message::builder()
							.title("Additional Output Difference".to_string())
							.description(format!("Desired <{}>, Given <{}>", s, ""))
							.build(),
					);
				}
				(None, Some(t)) => {
					messages.push(
                        run::Message::builder()
							.title("Additional Output Difference".to_string())
							.description(format!("Desired <{}>, Given <{}>", "", t))
							.build(),
					);
				}
				(None, None) => break,
			}
		}
	} else {
		messages.push(
            run::Message::builder()
				.title("未完成选做".to_string())
				.build(),
		);
	}

	Ok((passed, messages))
}
