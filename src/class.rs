use crate::assignment::Assignment;
use crate::student::Student;
use std::fmt::Display;
use std::path::Path;
use std::path;
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
			let student_ids: Vec<String> = class.students.iter()
				.map(|student| student.sis_login_id.to_string()).collect();
			class.assignments.iter().for_each(
				|assignment| {
					let group = assignment.group_by_student(&student_ids);
				}
			)
		});
		classes
	}

	pub fn load_class<P: AsRef<Path>>(path: P) -> Vec<Class> {
		let path = path.as_ref();
		let mut classes = Vec::new();
		for entry in path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let class = Class::builder()
					.name(path.file_stem().expect("file_stem failed").to_string_lossy().to_string())
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

	pub fn load_assignments(&mut self) {
		for entry in self.path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let assignment = Assignment::builder()
					.name(path.file_stem().expect("file_stem failed").to_string_lossy().to_string())
					.path(path)
					.build();
				self.assignments.push(assignment);
			}
		}
	}

	pub fn load_students(&mut self) {
		self.students = Student::load_from_roster(self.roster_path())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;


	#[test]
	fn test_load_class() {
		let config = Config::builder().build();
		assert_eq!(config.data_dir, path::Path::new("./data"));
		let classes = Class::load_class("./data");
		println!("{:?}", classes);
	}

	#[test]
	fn test_load_assignments() {
		let config = Config::builder().build();
		let classes = Class::load_class(&config.data_dir);
		let test_class = &classes[0];
		test_class.load_assignments();
		let assignments = test_class.assignments;
		println!("{:?}", assignments);
	}
}
