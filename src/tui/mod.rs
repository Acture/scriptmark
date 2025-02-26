use crate::defines::class;
use crate::defines::student::Student;
use crate::defines::submission_record::SubmissionRecord;
use dialoguer::{FuzzySelect, Select};
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use std::process::exit;

pub fn select_class(classes: &[class::Class]) {
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

				let submission_map =
					crate::check::check_assignment(selected_class, selected_assignment_name);
				select_submission(&submission_map);
			}
		}
	}
}

fn select_submission(results: &HashMap<Student, SubmissionRecord>) {
	let results_keys = results
		.keys()
		.sorted_by(|a, b| a.sis_login_id.cmp(&b.sis_login_id))
		.collect::<Vec<_>>();
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
			"返回" => return,
			"退出" => exit(0),
			_ => {
				let selected_record = results.get(results_keys[selected_record_index]).unwrap();
				select_detail(selected_record)
			}
		}
	}
}

fn select_detail(selected_record: &SubmissionRecord) {
	let options = std::iter::empty()
		.chain(
            selected_record
				.is_submitted
				.as_ref()
				.map(|_| "未提交")
				.into_iter(),
		)
		.chain(
            selected_record
				.has_hash_collision
				.as_ref()
				.map(|_| "查看hash冲突")
				.into_iter(),
		)
		.chain(selected_record.errors.as_ref().map(|_| "查看错误"))
		.chain(selected_record.messages.as_ref().map(|_| "查看消息"))
		.chain(["返回", "退出"].iter().copied())
		.collect::<Vec<_>>();

	loop {
		let selected = Select::new()
			.items(&options)
			.interact()
			.expect("Select failed");
		match options[selected] {
			"未提交" => break,
			"查看错误" => {
				select_error(selected_record);
			}
			"查看消息" => {
				select_message(selected_record);
			}
			"返回" => break,
			"退出" => exit(0),
			_ => panic!("未知选项"),
		}
	}
}

fn select_message(selected_record: &SubmissionRecord) {
	let messages = selected_record.messages.as_ref().unwrap();
	let mut message_options = messages.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>();
	message_options.extend(vec!["返回", "退出"]);
	loop {
		let selected_message_index = Select::new()
			.items(&message_options)
			.interact()
			.expect("Select failed");
		match message_options[selected_message_index] {
			"返回" => break,
			"退出" => exit(0),
			message_name => {
				if let Some(messages) = messages.get(message_name).unwrap() {
					println!("\t{} x {}:", message_name, messages.len());
					messages.iter().for_each(|msg| {
						println!("\t\t{}", msg);
					});
				};
			}
		}
	}
}

fn select_error(selected_record: &SubmissionRecord) {
	let errors = selected_record.errors.as_ref().unwrap();
	let error_options = errors
		.iter()
		.map(|(k, _)| k.as_str())
		.chain(["返回", "退出"].iter().copied())
		.collect::<Vec<_>>();
	loop {
		let selected_error_index = Select::new()
			.items(&error_options)
			.interact()
			.expect("Select failed");
		match error_options[selected_error_index] {
			"返回" => break,
			"退出" => exit(0),
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
