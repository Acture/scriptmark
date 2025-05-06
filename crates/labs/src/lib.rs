use common::defines::submission::Submission;
use common::defines::testresult::TestResult;
use common::rc_ref;
use common::traits::testsuite::DynTestSuite;
use log::debug;
use regex::Regex;
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

pub mod solutions;

pub fn parse_submission_file(path: &Path) -> Result<(String, String), Box<dyn std::error::Error>> {
	let file_name = path
		.file_name()
		.and_then(|os_str| os_str.to_str())
		.ok_or("Invalid or missing file name")?;

	let [sis_str, _, _, real_file_name_str]: [_; 4] = file_name
		.splitn(4, ['-', '_'])
		.collect::<Vec<_>>()
		.as_slice()
		.try_into()
		.map_err(|e| format!("Filename <{}> expected exactly 4 parts: {}", file_name, e))?;
	Ok((sis_str.to_string(), real_file_name_str.to_string()))
}
pub fn group_files_by_sis_id(path: &Path, extension: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
	let mut files_to_move = Vec::new();

	for entry in std::fs::read_dir(path)? {
		let entry = entry?;
		if entry.file_type()?.is_dir() {
			continue;
		}
		let file_path = entry.path();


		if let Some(ext) = extension.clone() {
			match file_path.extension() {
				Some(file_ext) => {
					if *file_ext != *ext {
						continue;
					}
				}
				None => {
					continue;
				}
			}
			let file_ext = file_path.extension().ok_or(format!("Failed to get file extension: {}", file_path.to_string_lossy()))?.to_str().ok_or("Invalid file extension")?;
			if file_ext != ext {
				continue;
			}
		}

		files_to_move.push(
			file_path
		)
	}

	for file in files_to_move {
		let (sis_id, filename) = parse_submission_file(&file)?;
		let student_folder = path.join(sis_id);
		if !student_folder.exists() {
			std::fs::create_dir_all(&student_folder)?;
		}
		std::fs::rename(&file, student_folder.join(filename))?;
	}

	Ok(())
}

pub fn find_test_suite(name: &str) -> Option<&dyn DynTestSuite> {
	solutions::TEST_SUITES
		.iter()
		.find(
			|dts| dts.get_name() == name
		)
		.map(|dts| {
			*dts as &dyn DynTestSuite
		})
}

pub fn run_test_suite(assignment_root: &Path, name: &str, pattern: &Regex) -> Vec<(String, Vec<Result<TestResult, String>>, Rc<RefCell<Submission>>)> {
	let test_suite = find_test_suite(name).expect("Test suite not found");

	let test_files = fs::read_dir(assignment_root).expect("Failed to read directory")
		.map(|assignment_entry| { assignment_entry.expect("Failed to read directory entry") })
		.filter(|assignment_entry| assignment_entry.path().is_dir())
		.flat_map(|student_assignment_dir| {
			fs::read_dir(student_assignment_dir.path()).expect("Failed to read directory")
				.filter_map(|submission_entry| submission_entry.ok())
				.map(|submission_entry| submission_entry.path())
				.collect::<Vec<_>>()
		})
		.filter_map(|submission_path| {
			let file_name = submission_path
				.file_stem().expect("Failed to get file stem")
				.to_str().expect("Failed to convert file stem to str");
			let sis_id = submission_path.parent().expect("Failed to get parent")
				.file_stem().expect("Failed to get file stem")
				.to_str().expect("Failed to convert file stem to str")
				.to_string();
			if pattern.is_match(file_name) {
				debug!("Found file {}", file_name);
				Some((sis_id, submission_path))
			} else {
				debug!("Ignoring file {}", file_name);
				None
			}
		})
		.collect::<Vec<_>>();

	println!("Found {} files", test_files.len());

	test_files.iter()
		.map(|(sis_id, test_file)| {
			let submission = rc_ref!(Submission::builder()
				.submission_path(test_file)
				.build());
			(sis_id.into(), test_suite.pipelined(test_file), submission)
		})
		.collect()
}

pub fn reg_of_possible_names(first: &str, second: Option<&str>) -> Regex {
	let reg_str = match second {
		Some(second) => format!(r"{}\s*[-._（(]\s*{}\s*[）)]?", first, second),
		None => first.to_string()
	};

	Regex::new(&reg_str).expect("Invalid regex")
}

#[cfg(test)]
mod tests {
	use super::*;
	use common::defines::student::Student;
	use std::collections::HashMap;
	use std::error::Error;

	#[test]
	fn test_valid_email() -> Result<(), Box<dyn Error>> {
		let test_suite = solutions::TEST_SUITES.iter().find(
			|dts| dts.get_name() == "valid_email"
		).expect("Test suite not found");

		let test_path = dev::env::DATA_DIR.clone()
			.join("COMP110042.09/作业8 (104838)/23307110287/23307110287_232949_5641683_Lab8_1.py");

		let _res = test_suite.pipelined(&test_path);

		Ok(())
	}

	#[test]
	fn test_group() -> Result<(), Box<dyn Error>> {
		let data_dir = dev::env::DATA_DIR.clone().join("COMP110042.09/作业9");
		group_files_by_sis_id(&data_dir, None)?;
		Ok(())
	}
	const TEST_SUITE_SCORE: usize = 40;


	#[test]
	fn test_run_test_suite() -> Result<(), Box<dyn Error>> {
		let class_id = "COMP110042.09";
		let assignment_id = "作业9";
		let data_dir = dev::env::DATA_DIR.clone().join(class_id).join(assignment_id);
		println!("{:?}", data_dir);
		let res = run_test_suite(&data_dir, "lower_case_to_upper_case", &reg_of_possible_names("9", None));
		let data_dir = dev::env::DATA_DIR.clone().join("test_save.json");
		let classes = common::dump::load_dump(&data_dir).expect("Failed to load");
		let comp_class = classes.iter().find(|c| { c.borrow().id == class_id }).expect("Failed to find class");

		let mut res = comp_class.borrow().students.iter().map(|s| {
			let sis_login_id = s.borrow().sis_login_id.clone(); // 提前取出

			let (res, res_sub) = {
				let found = res.iter().find(|(id, _, _)| id == &sis_login_id);
				(found.map(|(_, r, _)| r.to_owned()), found.map(|(_, _, s)| s))
			};

			(s.clone(), res, res_sub)
		}).collect::<Vec<_>>();

		res.sort_by_key(|(s, _, _)| {
			s.borrow().sis_login_id.clone()
		});
		let res: Vec<_> = res.into_iter().map(|(student, tr, tr_sub)| {
			let tr = tr.clone().unwrap_or_default();
			let test_count = tr.len();
			let tr_failed: Vec<_> = tr
				.into_iter()
				.filter(
					|tr|
						tr.is_err() || tr.as_ref().is_ok_and(|tr| !tr.passed)
				)
				.collect();


			let score = if test_count > 0 {
				TEST_SUITE_SCORE * (test_count - tr_failed.len()) / test_count
			} else {
				0 // 如果没有测试，得分为0
			};


			let mut hash_value = None;

			if let Some(sub) = tr_sub {
				let mut mut_sub = sub.borrow_mut();
				mut_sub.score = Some(score as f64);
				mut_sub.update_hash().expect("Failed to update hash");
				hash_value = mut_sub.cached_hash;
				student.borrow_mut().submissions.push(sub.clone());
			}


			(student, tr_failed, score, hash_value)
		}
		).collect();

		let grouped: HashMap<_, Vec<_>> = res
			.iter()
			.filter_map(|(s, _, _, hash_value)| hash_value.map(|h| (s.clone(), h)))
			.fold(HashMap::new(), |mut acc, (s, hash)| {
				acc.entry(hash).or_default().push(s);
				acc
			});


		for r in res.iter() {
			print_result(&r.0, &r.1, r.2, r.3, &grouped);
		}
		Ok(())
	}

	fn print_result(student: &Rc<RefCell<Student>>,
					tr_failed: &Vec<Result<TestResult, String>>,
					score_value: usize,
					hash_value: Option<u64>,
					grouped_hash: &HashMap<u64, Vec<Rc<RefCell<Student>>>>,
	) {
		let score = score_value;

		let _hash_collided = if let Some(hash) = hash_value {
			grouped_hash.get(&hash).map_or(vec![], |s| s.iter().map(|s| s.borrow().name.clone()).collect())
		} else { vec![] };
		let has_collided = match _hash_collided.len() > 1 {
			true => format!("{}: {:?}", "Hash Collided", _hash_collided),
			false => format!("{}: {:?}", "Not Collided", _hash_collided),
		};

		println!(	"{} - {} - {} - {:?} / {} - {:?} / 100",
					&student.borrow().name, &student.borrow().sis_login_id, has_collided, score, TEST_SUITE_SCORE, score + (100 - TEST_SUITE_SCORE));
		println!("\t{} - {:?} - {} / {}", "9", _hash_collided, score_value, 40);
		for test_result in tr_failed {
			println!("\t\t{:?}", test_result);
		}

		println!();
	}
}
