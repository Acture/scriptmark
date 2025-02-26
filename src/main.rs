use dialoguer::FuzzySelect;
use dialoguer::Select;
use env_logger;
use grade::circle_area;
use lazy_static::lazy_static;
use log::{info};
use std::collections::HashMap;
use std::env;
use student::Student;
use itertools::Itertools;
use submission_record::SubmissionRecord;

mod assignment;
mod class;
mod config;
mod grade;
mod run;
mod student;
mod submission_record;
mod utils;

lazy_static! {
	static ref CONFIG: config::Config = config::prepare_config();
}

fn init_logger() {
	env_logger::Builder::new()
		.parse_filters(&env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())) // 默认 info
		.init();
}

fn select_class(classes: &[class::Class]) {
	let class_options = classes
		.iter()
		.enumerate()
		.map(|(_i, class)| class.name.as_str())
		.chain(std::iter::once("退出"))
		.collect::<Vec<_>>();
	let class_selector = Select::new()
		.with_prompt("Please Select a Class")
		.items(&class_options);

	loop {
		let selected_index = class_selector.clone().interact().expect("Select failed");

		match class_options[selected_index] {
			"退出" => return,
			_ => {
				let selected_class = &classes[selected_index];
				info!("所选班级：{}", selected_class.name);
				select_assignment(selected_class);
			}
		}
	}
}

fn select_assignment(selected_class: &class::Class) {
	let assignment_options = selected_class
		.assignments
		.iter()
		.map(|assignment| assignment.name.as_str())
		.chain(vec!["返回", "退出"])
		.collect::<Vec<_>>();
	let assignment_selector = Select::new()
		.with_prompt("Please Select an Assignment")
		.items(&assignment_options);
	loop {
		let selected_index = assignment_selector
			.clone()
			.interact()
			.expect("Select failed");

		match assignment_options[selected_index] {
			"退出" => return,
			_ => {
				let selected_assignment_name = &assignment_options[selected_index];
				info!("所选作业：{}", selected_assignment_name);

				let submission_map = check_assignment(selected_class, selected_assignment_name);
				select_submission(&submission_map);
			}
		}
	}
}

fn check_assignment(
	selected_class: &class::Class,
	selected_assignment_name: &str,
) -> HashMap<Student, SubmissionRecord> {
	let student_assignments =
		selected_class.get_student_assignments(selected_assignment_name.to_string());
	let standard_py = CONFIG
		.data_dir
		.join(format!("{}.py", selected_assignment_name));
	if !standard_py.exists() {
		panic!("未找到标准答案: {:?}", standard_py);
	}
	let (_inputs, run_func, judge_func) = match selected_assignment_name {
		"lab1_circle_area" => (
			run::generate(CONFIG.seed, 20, 0.0, 100.0),
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
				.assignment(Some(selected_assignment_name.to_string()))
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
				submission_record.is_submitted = Some(false);

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
	results
}

fn select_submission(results: &HashMap<Student, SubmissionRecord>) {
    let results_keys = results.keys().sorted_by(|a, b| a.sis_login_id.cmp(&b.sis_login_id)).collect::<Vec<_>>();
	let record_options = results_keys
		.iter()
		.map(|student| -> std::string::String {
			let submission_record = results.get(student).unwrap();
			match submission_record.is_submitted {
				Some(false) => {
					format!("{} - {}: 未提交", student.name, student.sis_login_id)
				}
				_ => match (
					submission_record.errors.as_ref(),
					submission_record.messages.as_ref(),
				) {
					(Some(errors), Some(messages)) => {
						format!(
							"{} - {} : {} / {} has {} errors and {} messages",
							student.name,
							student.sis_login_id,
							submission_record
								.correct_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							submission_record
								.total_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							errors.len(),
							messages.len(),
						)
					}
					(Some(errors), None) => {
						format!(
							"{} - {} : {} / {} has {} errors",
							student.name,
							student.sis_login_id,
							submission_record
								.correct_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							submission_record
								.total_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							errors.len(),
						)
					}
					(None, Some(messages)) => {
						format!(
							"{} - {} : {} / {} has {} messages",
							student.name,
							student.sis_login_id,
							submission_record
								.correct_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							submission_record
								.total_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							messages.len(),
						)
					}
					(None, None) => {
						format!(
							"{} - {} : {} / {}",
							student.name,
							student.sis_login_id,
							submission_record
								.correct_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string()),
							submission_record
								.total_count
								.map(|c| c.to_string())
								.unwrap_or_else(|| "未知".to_string())
						)
					}
				},
			}
		})
		.chain(vec!["退出".to_string()])
		.collect::<Vec<_>>();
	loop {
		let selected_record_index = FuzzySelect::new()
			.with_prompt("Please Select a Record")
			.items(&record_options)
			.interact()
			.expect("Select failed");

		match record_options[selected_record_index].as_str() {
			"退出" => return,
			_ => {
				let selected_record = results.get(results_keys[selected_record_index]).unwrap();
				let mut options = Vec::new();
				if Some(false) == selected_record.is_submitted {
					println!("\t未提交");
				} else {
					if let Some(errors) = &selected_record.errors {
						options.push("查看错误");
					}
					if let Some(messages) = &selected_record.messages {
						options.push("查看消息");
					}
				}
				options.extend(vec!["返回", "退出"]);
				loop {
					let selected = Select::new()
						.items(&options)
						.interact()
						.expect("Select failed");
					match options[selected] {
						"查看错误" => {
							let errors = selected_record.errors.as_ref().unwrap();
							let mut error_options =
								errors.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>();
							error_options.extend(vec!["返回", "退出"]);
							loop {
								let selected_error_index = Select::new()
									.items(&error_options)
									.interact()
									.expect("Select failed");
								match error_options[selected_error_index] {
									"返回" => break,
									"退出" => return,
									error_name => {
										if let Some(error_details) = errors.get(error_name) {
											error_details.iter().for_each(|msg| {
												println!("{:?}", msg);
											});
										}
									}
								}
							}
						}
						"查看消息" => {
							let messages = selected_record.messages.as_ref().unwrap();
							let mut message_options =
								messages.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>();
							message_options.extend(vec!["返回", "退出"]);
							loop {
								let selected_message_index = Select::new()
									.items(&message_options)
									.interact()
									.expect("Select failed");
								match message_options[selected_message_index] {
									"返回" => break,
									"退出" => return,
									message_name => {
										if let Some(messages) = messages.get(message_name).unwrap()
										{
											println!("\t{} x {}:", message_name, messages.len());
											messages.iter().for_each(|msg| {
												println!("\t\t{}", msg);
											});
										};
									}
								}
							}
						}
						"返回" => break,
						"退出" => return,
						_ => panic!("未知选项"),
					}
				}
			}
		}
	}
}
fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let classes = class::Class::prepare_class(&CONFIG.data_dir);
	info!("班级信息加载完成");

	select_class(&classes);
}
