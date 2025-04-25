use crate::defines::assignment::Assignment;
use crate::defines::student::Student;
use crate::defines::submission::Submission;
use crate::traits::savenload::SaveNLoad;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Class {
	pub id: String,
	pub name: String,
	pub csv_path: PathBuf,
	pub class_path: PathBuf,
	#[builder(default)]
	pub students: Vec<Student>,
	#[builder(default)]
	pub assignments: Vec<Assignment>,
}

impl Display for Class {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Class {{ {} - {}, {} students, {} assignments }}", self.id, self.name, self.students.len(), self.assignments.len())
	}
}

impl SaveNLoad for Class {
	fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
		let file_name = path.to_path_buf().join(PathBuf::from(format!("{} - {}.json", self.id, self.name)));
		serde_json::to_writer_pretty(File::create(file_name)?, self)?;
		Ok(())
	}
	fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
		let class: Class = serde_json::from_reader(File::open(path)?)?;
		Ok(class)
	}
}


pub fn try_find_class_name_and_id_from_path(path: &Path) -> Result<(String, String), Box<dyn std::error::Error>> {
	let file_stem = path.file_stem().ok_or("Missing file stem in path")?.to_str().ok_or("File stem is not valid UTF-8")?;

	let parts: Vec<&str> = file_stem.split('_').collect();

	if parts.len() != 3 {
		return Err(format!("File name '{}' does not match expected format (e.g. prefix_class-123_name)", file_stem).into());
	}

	let class_id = parts[1].split('-').nth(1).ok_or("Missing '-' in class ID segment")?.to_string();

	let class_name = parts[2].to_string();

	Ok((class_name, class_id))
}


impl Class {
	pub fn prepare_class(p0: &Path) -> Vec<Class> {
		todo!()
	}
	pub fn parse_from_csv(	csv_path: PathBuf, class_dir: PathBuf,
							name: Option<&str>, id: Option<&str>, infer_from_path: bool)
							-> Result<Class, Box<dyn std::error::Error>> {
		let (name, id) = match (infer_from_path, name, id) {
			(true, _, _) => try_find_class_name_and_id_from_path(&csv_path)?,
			(false, Some(name), Some(id)) => (name.to_string(), id.to_string()),
			_ => panic!("Missing class name or ID")
		};
		let file = File::open(&csv_path)?;
		let mut rdr = csv::Reader::from_reader(file);
		let headers = rdr.headers()?.clone();
		let get_index = |h: &str| headers.iter().position(|x| x == h).ok_or(format!("Missing '{}' header", h));
		let (student_name_index, student_id_index, student_sis_login_id, score_range) = {
			let s = get_index("Student")?;
			let i = get_index("ID")?;
			let l = get_index("SIS Login ID")?;
			let section = get_index("Section")?;
			let points = get_index("作业 Current Points")?;
			(s, i, l, (section + 1)..points)
		};
		let points_possible_record = rdr.records().nth(0).ok_or("Missing points possible row")?.unwrap();
		if !points_possible_record.get(0).unwrap().contains("Points Possible") {
			return Err("Expected first row to be 'Points Possible' row".into());
		}
		let assignments: Vec<Assignment> = score_range.clone().map(|i| {
			let name = headers.get(i).unwrap().to_string();
			let score = points_possible_record.get(i).unwrap().parse::<f64>().unwrap();
			Assignment::builder()
				.name(name)
				.points_possible(score)
				.build()
		}).collect();
		let mut students = Vec::new();

		for result in rdr.records().skip(1) {
			let record = result?;
			let name = record.get(student_name_index).unwrap().to_string();
			if name == "测验学生" {
				continue;
			}
			let submissions: HashMap<String, Submission> = score_range
				.clone()
				.zip(assignments.iter())
				.filter_map(|(i, assignment)| {
					let score = record.get(i)?.parse::<f64>().ok()?;
					let Submission = Submission::builder().score(
						Some(score)
					).build();
					Some((assignment.name.clone(), Submission))
				})
				.collect();
			let student = Student::builder()
				.id(record.get(student_id_index).unwrap().to_string())
				.name(record.get(student_name_index).unwrap().to_string())
				.sis_login_id(record.get(student_sis_login_id).unwrap().to_string())
				.submissions(submissions)
				.build();
			students.push(
				student
			);
		};

		let class_path = class_dir.join(PathBuf::from(format!("{} - {}", id, name)));


		Ok(
			Class::builder()
				.id(id.to_string())
				.name(name.to_string())
				.csv_path(csv_path)
				.class_path(class_path)
				.students(students)
				.assignments(assignments)
				.build()
		)
	}

	pub fn create_dir_if_not_exists(&self) -> Result<(), Box<dyn std::error::Error>> {
		for assignment in &self.assignments {
			for task in &assignment.tasks {
				for student in &self.students {
					let dir = self.class_path
						.join(&assignment.name)
						.join(&task.name)
						.join(&student.id);
					if !dir.exists() {
						fs::create_dir_all(&dir)?; // 自动创建所有父目录
					}
				}
			}
		}
		Ok(())
	}
}


#[cfg(test)]
mod tests {
	use super::*;


	#[test]
	fn test_load_from_csv() {
		let data_dir = dev::env::DATA_DIR.to_path_buf();
		let test_csv_path = dev::env::DATA_DIR.to_path_buf().join("2025-04-25T0045_评分-AIB110002.13_Python程序设计.csv");
		let class = Class::parse_from_csv(test_csv_path, data_dir, None, None, true);
		assert!(class.is_ok());
		let class = class.unwrap();
		assert_eq!(class.id, "AIB110002.13");
		let class_save = dev::env::DATA_DIR.to_path_buf();
		class.save(&class_save).expect("TODO: panic message");
		println!("{}", class);
	}

	#[test]
	fn test_save_load() {
		let data_dir = dev::env::DATA_DIR.to_path_buf();
		let class_dir = data_dir.join("AIB110002.13 - Python程序设计.json");
		let loaded_class = Class::load(&class_dir).unwrap();
		let test_csv_path = data_dir.join("2025-04-25T0045_评分-AIB110002.13_Python程序设计.csv");
		let parsed_class = Class::parse_from_csv(test_csv_path, data_dir, None, None, true).unwrap();
		assert_eq!(loaded_class, parsed_class);
	}

	#[test]
	fn test_create_dir() {
		let data_dir = dev::env::DATA_DIR.to_path_buf();
		let class_dir = data_dir.join("AIB110002.13 - Python程序设计.json");
		let loaded_class = Class::load(&class_dir).unwrap();
		loaded_class.create_dir_if_not_exists().unwrap();
	}
}
