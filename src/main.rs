use dialoguer::Select;
use env_logger;
use grade::circle_area;
use log::{info, warn};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use student::Student;
use submission_record::SubmissionRecord;
mod assignment;
mod class;
mod config;
mod grade;
mod run;
mod student;
mod submission_record;
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
	let classes = class::Class::prepare_class(&config.data_dir);
	info!("班级信息加载完成");
	let class_options = classes
		.iter()
		.enumerate()
		.map(|(_i, class)| class.name.as_str())
		.collect::<Vec<_>>();
	let selected_index = Select::new()
		.with_prompt("Please Select a Class:")
		.items(&class_options)
		.interact()
		.expect("Select failed");

	let selected_class = &classes[selected_index];
	info!("所选班级：{}", selected_class.name);

	let assignment_options = selected_class
		.assignments
		.iter()
		.map(|assignment| assignment.name.as_str())
		.collect::<Vec<_>>();
	let selected_assignment_index = Select::new()
		.with_prompt("Please Select an Assignment:")
		.items(&assignment_options)
		.interact()
		.expect("Select failed");
	let selected_assignment_name = &selected_class.assignments[selected_assignment_index].name;
	info!("所选作业：{}", selected_assignment_name);

	info!("开始检查作业...");
	let student_assignments =
		selected_class.get_student_assignments(selected_assignment_name.to_string());

	let standard_py = config
		.data_dir
		.join(format!("{}.py", selected_assignment_name));
	if !standard_py.exists() {
		warn!("未找到标准答案: {:?}", standard_py);
		return;
	}
	let (_inputs, run_func, judge_func) = match selected_assignment_name.as_str() {
		"lab1" => (
			run::generate(config.seed, 20, 0.0, 100.0),
			circle_area::run_lab_one,
			circle_area::judge_func,
		),
		_ => panic!("未知作业"),
	};

	let standard = run_func(&standard_py, &_inputs);

	let keys = {
		let mut k = student_assignments.keys().collect::<Vec<_>>();
		k.sort_unstable_by_key(|s| &s.sis_login_id);
		k
	};

	let mut hash_record: HashMap<u64, Vec<Student>> = HashMap::new();

	let bar = indicatif::ProgressBar::new(keys.len() as u64);

	let mut results: HashMap<Student, SubmissionRecord> = keys
		.iter()
		.map(|student| {
			let mut submission_record = SubmissionRecord::builder()
				.student(Some((*student).clone()))
				.assignment(Some(selected_assignment_name.clone()))
				.build();
			let file_paths = match student_assignments.get(student) {
				Some(paths) => paths,
				None => {
					submission_record
						.errors
						.get_or_insert_with(HashMap::new)
						.entry("未找到学生".to_string())
						.or_insert(None);
					return ((*student).clone(), submission_record);
				}
			};

			if file_paths.len() == 0 {
				submission_record
					.errors
					.get_or_insert_with(HashMap::new)
					.entry("未提交".to_string())
					.or_insert(None);

				return ((*student).clone(), submission_record);
			}
			if file_paths.len() > 1 {
				submission_record
					.messages
					.get_or_insert_with(HashMap::new)
					.entry("提交了多个文件".to_string())
					.or_insert(None);
			}
			let file_path = &file_paths[0];
			let hash = match run::calculate_hash_from_file(file_path) {
				Ok(hash) => hash,
				Err(e) => {
					submission_record
						.errors
						.get_or_insert_with(HashMap::new)
						.entry("计算hash失败".to_string())
						.or_insert(None);
					return ((*student).clone(), submission_record);
				}
			};
			match hash_record.entry(hash) {
				std::collections::hash_map::Entry::Occupied(mut entry) => {
					let students = entry.get_mut();
					students.push((*student).clone());
				}
				std::collections::hash_map::Entry::Vacant(entry) => {
					entry.insert(vec![(*student).clone()]);
				}
			}

			let to_be_judged = run_func(file_path, &_inputs);

			let judge_result = run::judge(&standard, &to_be_judged, Some(judge_func));
			submission_record.total_count = Some(judge_result.len());
			submission_record.correct_count = Some(
                judge_result
					.iter()
					.filter(|(is_correct, _)| *is_correct)
					.count(),
			);
			judge_result.iter().for_each(|(_is_correct, msgs)| {
				msgs.iter().for_each(|msg| match msg.title.as_str() {
					"完成选做" => {
						submission_record.did_additional = Some(true);
					}
					"未完成选做" => {
						submission_record.did_additional = Some(false);
					}

					_ => {
						submission_record
							.messages
							.get_or_insert_with(HashMap::new)
							.entry(msg.title.clone())
							.or_insert_with(|| Some(Vec::new()))
							.get_or_insert_with(Vec::new)
							.push(msg.description.clone());
					}
				});
			});
			bar.inc(1);
			((*student).clone(), submission_record)
		})
		.collect();
	bar.finish();

	hash_record
		.iter()
		.filter(|(_, students)| students.len() > 1)
		.for_each(|(hash, collided_students)| {
			collided_students.iter().for_each(|collided_student| {
				results
					.get_mut(collided_student)
					.ok_or("未找到学生")
					.unwrap()
					.has_hash_collision
					.get_or_insert_with(Vec::new)
					.extend(
                        collided_students
							.iter()
							.map(|s| s.name.clone())
							.collect::<Vec<_>>(),
					);
			})
		});

	results.iter().for_each(|(student, submission_record)| {
		println!("学生: {}", student.name);
		println!("Submission Record: {:#?}", submission_record);
	});
}
