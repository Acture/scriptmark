use crate::check;
use crate::defines::class;
use crate::defines::student::Student;
use dialoguer::{FuzzySelect, Select};
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use std::process::exit;
use suite::test_suite::TestResult;

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
				println!("{:?}", submission_map);
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
			format!(
				"{} - {}\t{}/{}",
				student.name,
				student.sis_login_id,
				pass_count,
				record.len()
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

fn select_detail(selected_record: &Vec<TestResult>) {}
