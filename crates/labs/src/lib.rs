use common::defines::testresult::TestResult;
use common::traits::testsuite::DynTestSuite;
use std::fs;
use std::ops::Deref;
use std::path::Path;

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
			let file_ext = file_path.extension().ok_or("Failed to get file extension")?.to_str().ok_or("Invalid file extension")?;
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
		std::fs::copy(&file, student_folder.join(filename))?;
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

pub fn run_test_suite(assignment_root: &Path, name: &str, pattern: &str) -> Vec<(String, Vec<Result<TestResult, String>>)> {
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
			let (sis_id, _) = parse_submission_file(&submission_path).ok()?;
			let pattern = regex::Regex::new(pattern).expect("Invalid regex");
			if pattern.is_match(file_name) {
				Some((sis_id, submission_path))
			} else {
				None
			}
		})
		.collect::<Vec<_>>();

	test_files.iter()
		.map(|(sis_id, test_file)| {
			(sis_id.into(), test_suite.pipelined(test_file))
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::error::Error;

	#[test]
	fn test_valid_email() -> Result<(), Box<dyn Error>> {
		let test_suite = solutions::TEST_SUITES.iter().find(
			|dts| dts.get_name() == "valid_email"
		).expect("Test suite not found");

		let test_path = dev::env::DATA_DIR.clone()
			.join("COMP110042.09/作业8 (104838)/23307110287/23307110287_232949_5641683_Lab8_1.py");

		let res = test_suite.pipelined(&test_path);

		Ok(())
	}

	#[test]
	fn test_group() -> Result<(), Box<dyn Error>> {
		let data_dir = dev::env::DATA_DIR.clone().join("COMP110042.09/作业8 (104838)");
		group_files_by_sis_id(&data_dir, Some("py".to_string()))?;
		Ok(())
	}

	#[test]
	fn test_run_test_suite() -> Result<(), Box<dyn Error>> {
		let data_dir = dev::env::DATA_DIR.clone().join("COMP110042.09/作业8 (104838)");
		let res = run_test_suite(&data_dir, "valid_email", r"8\s*[-._（(]\s*1\s*[）)]?\s*$");

		for (sis_id, test_results) in res {
			let failed_test_results = test_results.iter()
				.filter(|&test_result| test_result.is_err() || test_result.clone().is_ok_and(|t| { !t.passed }))
				.collect::<Vec<_>>();
			println!("{} - {} / {}", sis_id, test_results.len() - failed_test_results.len(), test_results.len());

			for test_result in failed_test_results {
				println!("\t{:?}", test_result);
			}
		}
		Ok(())
	}
}
