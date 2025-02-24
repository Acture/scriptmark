use crate::grade::circle_area;
use crate::student::Student;
use env_logger;
use grade::circle_area;
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

fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let config = config::prepare_config();
	let mut classes = class::Class::prepare_class(&config.data_dir);
	info!("班级信息加载完成");
	let test_class = &mut classes[0];
	let student_assignments = test_class.get_student_assignments("lab1".to_string());

	let lab1_path = config.data_dir.join("lab1.py");
	let test_inputs = run::generate_twentys(42);
	let standard = grade::circle_area::run_lab_one(lab1_path, &test_inputs);

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
		let hash =
			run::calculate_hash_from_file(file_path).expect("calculate_hash_from_file failed");
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
		let to_be_judged = circle_area::run_lab_one(file_path, &test_inputs);

		let res = run::judge(&standard, &to_be_judged, Some(circle_area::judge_func));
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
			msgs.iter().filter(|msg| !msg.is_empty()).for_each(|msg| {
				info!("\t\t{}", msg);
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
