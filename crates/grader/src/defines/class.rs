use crate::defines::assignment::Assignment;
use crate::defines::student::Student;
use log::warn;
use std::collections::HashMap;
use std::path;
use std::path::{Path, PathBuf};
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct Class {
	pub name: String,
	pub path: path::PathBuf,
	#[builder(default = Vec::new())]
	pub students: Vec<Student>,
	#[builder(default = Vec::new())]
	pub assignments: Vec<Assignment>,
}

impl Class {
	pub fn prepare_class<P: AsRef<Path>>(path: P) -> Vec<Class> {
		let mut classes = Class::load_class(path);
		classes.iter_mut().for_each(|class| {
			class.load_students();
			class.load_assignments();
			class.assignments.iter().for_each(|assignment| {
				let _group = assignment.group_by_student(&class.students);
			})
		});
		classes
	}

	pub(crate) fn load_class<P: AsRef<Path>>(path: P) -> Vec<Class> {
		let path = path.as_ref();
		let mut classes = Vec::new();
		for entry in path
			.read_dir()
			.expect(format!("Read Path <{}> failed.", path.display()).as_str())
		{
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let class = Class::builder()
					.name(
                        path.file_stem()
							.expect("file_stem failed")
							.to_string_lossy()
							.to_string(),
					)
					.path(path)
					.build();
				classes.push(class);
			}
		}
		classes
	}

	pub fn roster_path(&self) -> path::PathBuf {
		self.path.join("roster.csv")
	}

	pub(crate) fn load_assignments(&mut self) {
		for entry in self.path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let assignment = Assignment::builder()
					.name(
                        path.file_stem()
							.expect("file_stem failed")
							.to_string_lossy()
							.to_string(),
					)
					.path(path)
					.build();
				self.assignments.push(assignment);
			}
		}
	}

	pub(crate) fn load_students(&mut self) {
		self.students = Student::load_from_roster(self.roster_path())
	}

	pub fn get_student_assignments(
		&self,
		assignment_name: String,
	) -> HashMap<Student, Vec<PathBuf>> {
		let assignment = self
			.assignments
			.iter()
			.find(|a| a.name == assignment_name)
			.unwrap_or_else(|| panic!("{} - {} 未找到作业", self.name, assignment_name));
		assignment
			.group_by_student(&self.students)
			.iter()
			.filter_map(|(id, file_paths)| {
				match self
					.students
					.iter()
					.find(|student: &&Student| student.sis_login_id == *id)
				{
					Some(student) => Some((student.clone(), file_paths.clone())),
					None => {
						warn!("未找到对应学生 - {}", id);
						None
					}
				}
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;
	use std::fs;

	#[test]
	fn test_load_class() {
		let config = Config::builder()
			.data_dir(path::Path::new("../../data").to_path_buf())
			.build();
		println!("{:?}", fs::canonicalize(config.data_dir.clone()));
		let classes = Class::load_class(&config.data_dir);
		println!("{:?}", classes);
	}

	#[test]
	fn test_load_assignments() {
		let config = Config::builder()
			.data_dir(path::Path::new("../../data").to_path_buf())
			.build();
		let mut classes = Class::load_class(&config.data_dir);
		let test_class = &mut classes[0];
		test_class.load_assignments();
		let assignments = &test_class.assignments;
		println!("{:?}", assignments);
	}

	#[test]
	fn test_get_student_assignments() {
		let config = Config::builder()
			.data_dir(path::Path::new("../../data").to_path_buf())
			.build();
		let mut classes = Class::prepare_class(&config.data_dir);
		let test_class = &mut classes[0];
		let student_assignments = test_class.get_student_assignments("lab1_population".to_string());
		println!("{:?}", student_assignments);
	}
}
