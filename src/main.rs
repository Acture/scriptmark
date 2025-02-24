use crate::grade::lab1;
use crate::student::Student;
use env_logger;
use log::{info, warn};
use std::collections::HashMap;
use std::env;
use std::error::Error;

mod assignment;
mod class;
mod config;
mod grade;
mod run;
mod student;
mod utils;

fn init_logger() {
	env_logger::Builder::new()
		.parse_filters(&env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())) // 默认 info
		.init();
}

fn judge_func(s: &str, t: &str) -> Result<(bool, Vec<lab1::Message>), (bool, Vec<lab1::Message>)> {
	let s_lines: Vec<_> = s.split("\n").collect();
	let t_lines: Vec<_> = t.split("\n").collect();
	let required_s: f64 = s_lines
		.get(0)
		.ok_or_else(|| {
			(
				false,
				vec![lab1::Message::builder()
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
				vec![lab1::Message::builder()
					.title("Standard Parse Error".to_string())
					.build()],
			)
		})?;
	let required_t: f64 = t_lines
		.get(0)
		.ok_or_else(|| {
			(
				false,
				vec![lab1::Message::builder()
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
				vec![lab1::Message::builder()
					.title("Tested Parse Error".to_string())
					.description(format!("The tested value is {}. Expected {}", t, required_s))
					.build()],
			)
		})?;

	let passed;
	let mut messages = Vec::new();

	if required_s != required_t {
		let diff = ((required_s - required_t) / required_s).abs() * 100.0;
		if diff < 1.0 {
			let mut message = lab1::Message::builder()
				.title("Small Value Difference".to_string())
				.build();
			message
				.description
				.push_str(format!("The difference is {} %", diff).as_str());
			passed = true;
			messages.push(message);
		} else {
			passed = false;
			let mut message = lab1::Message::builder()
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
	if additional_t.len() == 0 {
		messages.push(
            lab1::Message::builder()
				.title("没有选做".to_string())
				.build(),
		);
	} else {
		let mut additional_s = additional_s.iter();
		let mut additional_t = additional_t.iter();
		messages.push(
			lab1::Message::builder()
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
                            lab1::Message::builder()
								.title("Additional Output Difference".to_string())
								.description(format!("Desired <{}>, Given <{}>", s, t))
								.build(),
						);
					}
				}
				(Some(s), None) => {
					messages.push(
                        lab1::Message::builder()
							.title("Additional Output Difference".to_string())
							.description(format!("Desired <{}>, Given <{}>", s, ""))
							.build(),
					);
				}
				(None, Some(t)) => {
					messages.push(
                        lab1::Message::builder()
							.title("Additional Output Difference".to_string())
							.description(format!("Desired <{}>, Given <{}>", "", t))
							.build(),
					);
				}
				(None, None) => break,
			}
		}
	}

	Ok((passed, messages))
}

fn calculate_hash(s: &str) -> u64 {
	use std::collections::hash_map::DefaultHasher;
	use std::hash::{Hash, Hasher};
	let mut hasher = DefaultHasher::new();
	s.hash(&mut hasher);
	hasher.finish()
}

fn calculate_hash_from_file(file_path: &std::path::Path) -> Result<u64, Box<dyn Error>> {
	let content = std::fs::read_to_string(file_path)?;
	Ok(calculate_hash(&content))
}

fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let config = config::prepare_config();
	let mut classes = class::Class::prepare_class(&config.data_dir);
	info!("班级信息加载完成");
	let test_class = &mut classes[0];
	let student_assignments = test_class.get_student_assignments("lab1".to_string());

	let lab1_path = config.data_dir.join("lab1.py");
	let test_inputs = grade::lab1::generate_twentys(42);
	let standard = grade::lab1::run_lab_one(lab1_path, &test_inputs);

	let mut keys: Vec<_> = student_assignments.keys().collect();
	keys.sort_by(|a, b| a.sis_login_id.cmp(&b.sis_login_id));

	let mut hash_record: HashMap<u64, Vec<Student>> = HashMap::new();

	keys.iter().for_each(|student| {
		let file_paths = student_assignments.get(student).unwrap();
		if file_paths.len() == 0 {
			warn!("学生 {} {} 未提交作业", student.name, student.sis_login_id);
			return;
		}
		if file_paths.len() > 1 {
			warn!("学生 {} 提交了多个文件，只使用第一个文件", student.name);
		}
		let file_path = &file_paths[0];
		let hash = calculate_hash_from_file(file_path).expect("calculate_hash_from_file failed");
		match hash_record.entry(hash) {
			std::collections::hash_map::Entry::Occupied(mut entry) => {
				let students = entry.get_mut();
				students.push((*student).clone());
				warn!(
					"学生 {} {} 提交了相同的作业",
					student.name, student.sis_login_id
				);
			}
			std::collections::hash_map::Entry::Vacant(entry) => {
				entry.insert(vec![(*student).clone()]);
			}
		}
		let to_be_judged = lab1::run_lab_one(file_path, &test_inputs);

		let res = grade::lab1::judge(&standard, &to_be_judged, Some(judge_func));
		info!(
			"学生 {} {} 的成绩 {} / {}:",
			student.name,
			student.sis_login_id,
			res.iter().filter(|(is_correct, _)| *is_correct).count(),
			res.len()
		);
		let mut message_record = HashMap::new();
		res.iter().for_each(|(_is_correct, msgs)| {
			msgs.iter().for_each(|msg| {
				message_record
					.entry(msg.title.clone())
					.or_insert(Vec::new())
					.push(msg.description.clone());
			});
		});
		message_record.iter().for_each(|(msg_title, msgs)| {
			info!("\t {} x {}", msg_title, msgs.len());
			msgs.iter().for_each(|msg| {
				info!("\t\t {}", msg);
			});
		});
	});

	info!("以下学生提交了相同hash的作业:");
	hash_record
		.iter()
		.filter(|(_, students)| students.len() > 1)
		.for_each(|(hash, students)| {
			info!("\tHash: {}", hash);
			students.iter().for_each(|student| {
				info!("\t\t {} {}", student.name, student.sis_login_id);
			});
		});
}
