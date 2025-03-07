use crate::defines::class;
use crate::defines::student::Student;
use lab::TestSuiteType;
use log::{debug};
use std::collections::HashMap;
use suite::test_suite::TestResult;

pub fn check_assignment(
	selected_class: &class::Class,
	selected_assignment_name: &str,
	allow_custom: bool,
) -> (
	HashMap<Student, Vec<TestResult>>,
	HashMap<u64, Vec<Student>>,
) {
	let student_assignments =
		selected_class.get_student_assignments(selected_assignment_name.to_string());

	let select_assignment_type = match TestSuiteType::from_endwith(selected_assignment_name){
		Some(select_assignment_type) => select_assignment_type,
		None => {
			if allow_custom {
				unimplemented!("自定义测试套件");
			} else {
				panic!("未找到对应的测试套件");
			}
		}
	};

	let solution_map = &lab::TEST_SUITE_MAP;
	let test_suite = solution_map
		.get(&select_assignment_type)
		.expect("未找到测试套件");

	let mut hash_map: HashMap<u64, Vec<Student>> = HashMap::new();

	let assignments = student_assignments
		.iter()
		.map(|(student, files)| {
			match files.len() {
				0 => {
					debug!(
						"{} - {}: 未找到作业文件",
						student.name, student.sis_login_id
					);
					return (student.clone(), vec![]);
				}
				1 => (),
				_ => {
					debug!(
						"{} - {}: 作业文件数量不为1",
						student.name, student.sis_login_id
					);
					debug!("{:?}", files)
				}
			}

			let file = files.first().expect("Failed to get file");
			let file_hash = util::calculate_hash_from_file(file).expect("Failed to calculate hash");
			hash_map
				.entry(file_hash)
				.or_default()
				.push(student.clone());
			let result = test_suite.run_any(file);
			let answer = test_suite.get_answer_any();

			(
				student.clone(),
				test_suite.judge_any(result.as_ref(), answer.as_ref()),
			)
		})
		.collect::<HashMap<Student, Vec<TestResult>>>();
	(assignments, hash_map)
}
