use crate::check;
use crate::defines::class;
use crate::defines::student::Student;
use dialoguer::{FuzzySelect, Select};
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use std::os::macos::raw::stat;
use std::process::exit;
use std::string::String;
use suite::test_suite::{AdditionalStatus, TestResult};

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
					check::check_assignment(selected_class, selected_assignment_name);
				select_submission(&submission_map);
			}
		}
	}
}

fn select_submission(results: &HashMap<Student, Vec<TestResult>>) {
	let results_keys = results
		.keys()
		.sorted_by(|a, b| a.sis_login_id.cmp(&b.sis_login_id))
		.collect::<Vec<_>>();
	let record_options = results_keys
		.iter()
		.map(|student| -> std::string::String {
			let record = results.get(student).expect("Failed to get test result");
			let pass_count = record.iter().filter(|r| r.passed).count();
			let info_count = record.iter().filter(|r| r.infos.is_some()).count();
			let add_info_count = record.iter().filter(|r| r.infos.is_some()).count();

			let additional_status = if record.is_empty() {
				AdditionalStatus::None
			} else {
				record
					.iter()
					.map(|r| {
						r.additional_status
							.as_ref()
							.unwrap_or(&suite::test_suite::AdditionalStatus::None)
					})
					.fold(AdditionalStatus::Full, |acc, status| {
						if *status == AdditionalStatus::Partial || acc == AdditionalStatus::Partial
						{
							AdditionalStatus::Partial
						} else if *status == AdditionalStatus::None || acc == AdditionalStatus::None
						{
							AdditionalStatus::None
						} else {
							AdditionalStatus::Full
						}
					})
			};

			format!(
				"{:<10}\t{:<10}\t{:>2}/{:>}\t{}\t{:>2} infos\t{:>2} add_infos",
				student.name,
				student.sis_login_id,
				pass_count,
				record.len(),
				additional_status.to_string(),
				info_count,
				add_info_count,
			)
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

fn select_detail(selected_record: &[TestResult]) {
	let result_options = selected_record
		.iter()
		.flat_map(|result| {
			let info_keys = if let Some(infos) = &result.infos {
				infos.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<String>>()
			} else {
				Vec::new()
			}.into_iter();

			let additional_keys = if let Some(additional_infos) = &result.additional_infos {
				additional_infos.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<String>>()
			} else {
				Vec::new()
			}.into_iter();

			info_keys.chain(additional_keys).collect::<Vec<_>>()
		})
		.chain(std::iter::once("返回".to_string()))
		.chain(std::iter::once("退出".to_string()))
		.collect::<Vec<_>>();

	loop {
		let selected_result_index = FuzzySelect::new()
			.default(0)
			.with_prompt("Please Select a Result")
			.items(&result_options)
			.interact()
			.expect("Select failed");

		let selected_result_keys = result_options[selected_result_index].clone();

		match selected_result_keys.as_str() {
			"返回" => return,
			"退出" => exit(0),
			_ => {
				info!("Result: {:?}", selected_record[selected_result_index]);
			}
		}
	}
}
