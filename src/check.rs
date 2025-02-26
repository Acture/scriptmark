use crate::defines::class;
use crate::defines::student::Student;
use crate::defines::submission_record::SubmissionRecord;
use crate::lab::circle_area;
use crate::run;
use crate::CONFIG;
use std::collections::HashMap;

pub fn check_assignment(
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
