use common::defines::class;
use common::defines::student::Student;
use common::defines::test_suite::{AdditionalStatus, TestResult};
use dialoguer::{FuzzySelect, Select};
use itertools::Itertools;
use log::info;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)] // 让它支持 Debug 和比较
pub enum SelectStatus {
	Selected,
	Return,
	Exit,
}

pub fn select_class(classes: &[class::Class]) -> (SelectStatus, Option<&class::Class>) {
	let class_options = classes
		.iter()
		.map(|class| class.name.as_str())
		.chain(std::iter::once("退出"))
		.collect::<Vec<_>>();
	let class_selector = Select::new()
		.with_prompt("Please Select a Class")
		.items(&class_options);
	let selected_index = class_selector.clone().interact().expect("Select failed");

	match class_options[selected_index] {
		"退出" => (SelectStatus::Exit, None),
		_ => {
			let selected_class = &classes[selected_index];
			info!("所选班级：{}", selected_class.name);
			(SelectStatus::Selected, Some(selected_class))
		}
	}
}

pub fn select_assignment(selected_class: &class::Class) -> (SelectStatus, Option<String>) {
	let assignment_options = selected_class
		.assignments
		.iter()
		.map(|assignment| assignment.name.as_str())
		.chain(vec!["返回", "退出"])
		.collect::<Vec<_>>();
	let assignment_selector = Select::new()
		.with_prompt("Please Select an Assignment")
		.items(&assignment_options);
	let selected_index = assignment_selector
		.clone()
		.interact()
		.expect("Select failed");

	match assignment_options[selected_index] {
		"退出" => (SelectStatus::Exit, None),
		"返回" => (SelectStatus::Return, None),
		_ => {
			let selected_assignment_name = &assignment_options[selected_index];
			info!("所选作业：{}", selected_assignment_name);
			(
				SelectStatus::Selected,
				Some(selected_assignment_name.to_string()),
			)
		}
	}
}

pub fn select_test_result<'a>(
	submissions: &'a mut HashMap<Student, Vec<TestResult>>,
	_hash_map: &'a HashMap<u64, Vec<Student>>,
) -> (SelectStatus, Option<&'a [TestResult]>) {
	let results_keys = submissions
		.keys()
		.cloned()
		.sorted_by(|a, b| a.sis_login_id.cmp(&b.sis_login_id))
		.collect::<Vec<_>>();
	let record_options = results_keys
		.iter()
		.map(|student| -> std::string::String {
			let record: &mut Vec<TestResult> = submissions
				.get_mut(student)
				.expect("Failed to get test result");

			let collided_students: Vec<_> = _hash_map
				.iter()
				.filter(|(_, v)| v.len() > 1 && v.contains(student))
				.flat_map(|(_hash, v)| {
					v.iter()
						.filter(|s| s.sis_login_id != student.sis_login_id)
						.map(|s| format!("{} - {}", s.name, s.sis_login_id))
						.collect::<Vec<_>>()
				})
				.collect();

			let collide_test_result = match collided_students.is_empty() {
				true => TestResult {
					passed: true,
					infos: None,
					additional_infos: None,
					additional_status: Some(AdditionalStatus::None),
				},
				false => TestResult {
					passed: false,
					infos: Some(HashMap::from([(
						"Hash Collision detected".to_string(),
						format!(
							"Collided with {} students: {}",
							collided_students.len(),
							collided_students.join(", ")
						),
					)])),
					additional_infos: None,
					additional_status: Some(AdditionalStatus::None),
				},
			};

			let hash_collision_status = !collide_test_result.passed;

			if !record.contains(&collide_test_result) {
				record.push(collide_test_result);
			}
			let pass_count = record.iter().filter(|r| r.passed).count();
			let info_count = record.iter().filter(|r| r.infos.is_some()).count();
			let add_info_count = record
				.iter()
				.filter(|r| r.additional_infos.is_some())
				.count();

			let additional_status = if record.is_empty() {
				AdditionalStatus::None
			} else {
				record
					.iter()
					.map(|r| {
						r.additional_status
							.as_ref()
							.unwrap_or(&common::defines::test_suite::AdditionalStatus::None)
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
				"{:<10}\t{:<10}\t{:<4}\t{:>4}\t{:>2}/{:>2}\t{:<5}\t{:>2} infos\t{:>2} add_infos",
				student.name,
				student.sis_login_id,
				match record.len() > 1 {
					true => "已提交",
					false => "未提交",
				},
				match hash_collision_status {
					true => "冲突",
					false => "无冲突",
				},
				pass_count,
				record.len(),
				match additional_status {
					AdditionalStatus::None => "未完成附加",
					AdditionalStatus::Partial => "尝试附加",
					AdditionalStatus::Full => "完成附加",
				},
				info_count,
				add_info_count,
			)
		})
		.chain(vec!["返回".to_string(), "退出".to_string()])
		.collect::<Vec<_>>();
	let selected_record_index = FuzzySelect::new()
		.with_prompt("Please Select a Record")
		.items(&record_options)
		.interact()
		.expect("Select failed");

	match record_options[selected_record_index].as_str() {
		"返回" => (SelectStatus::Return, None),
		"退出" => (SelectStatus::Exit, None),
		_ => {
			let selected_test_result = submissions
				.get(&results_keys[selected_record_index])
				.expect("Failed to get test result");
			(SelectStatus::Selected, Some(selected_test_result))
		}
	}
}

pub fn select_detail(selected_record: &[TestResult]) -> (SelectStatus, Option<String>) {
	let mut result_options = Vec::new();

	// 遍历处理每个测试结果
	for (i, result) in selected_record.iter().enumerate() {
		// 处理普通信息
		if let Some(infos) = &result.infos {
			for (k, v) in infos.iter() {
				result_options.push(format!("{} - {} - {}", i, k, v));
			}
		}

		// 处理附加信息
		if let Some(additional_infos) = &result.additional_infos {
			for (k, v) in additional_infos.iter() {
				result_options.push(format!("{} - {} - {}", i, k, v));
			}
		}
	}

	// 添加返回和退出选项
	result_options.push("返回".to_string());
	result_options.push("退出".to_string());

	let selected_result_index = FuzzySelect::new()
		.default(0)
		.with_prompt("Please Select a Result")
		.items(&result_options)
		.interact()
		.expect("Select failed");

	let selected_result_keys = result_options[selected_result_index].clone();

	match selected_result_keys.as_str() {
		"返回" => (SelectStatus::Return, None),
		"退出" => (SelectStatus::Exit, None),
		_ => (SelectStatus::Selected, None),
	}
}
