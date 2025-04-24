use crate::defines::assignment::Assignment;
use crate::defines::student::Student;
use crate::traits::savable::Savable;
use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::path::{Path, PathBuf};
use typed_builder::TypedBuilder;
#[derive(TypedBuilder, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Class {
	pub id: String,
	pub name: String,
	pub submission_path: PathBuf,
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


pub fn try_find_class_name_and_id_from_path(path: &Path) -> Result<(String, String), Box<dyn std::error::Error>> {
	let file_stem = path.file_stem().ok_or("Missing file stem in path")?.to_str().ok_or("File stem is not valid UTF-8")?;

	let parts: Vec<&str> = file_stem.split('_').collect();

	if parts.len() != 3 {
		warn!("File name '{}' does not match expected format (e.g. prefix_class-123_name)", file_stem);
		return Err("Invalid file name format".into());
	}

	let class_id = parts[1].split('-').nth(1).ok_or("Missing '-' in class ID segment")?.to_string();

	let class_name = parts[2].to_string();

	Ok((class_name, class_id))
}


impl Class {
	pub fn prepare_class(p0: &Path) -> Vec<Class> {
		todo!()
	}
	pub fn load_from_csv(	csv_path: PathBuf,
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
			let submissions: HashMap<String, f64> = score_range
				.clone()
				.zip(assignments.iter())
				.filter_map(|(i, assignment)| {
					let score = record.get(i)?.parse::<f64>().ok()?;
					Some((assignment.name.clone(), score))
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


		Ok(
			Class::builder()
				.id(id.to_string())
				.name(name.to_string())
				.submission_path(csv_path)
				.students(students)
				.assignments(assignments)
				.build()
		)
	}
}

impl Savable for Class {
	fn save(&self, save_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
		let file = std::fs::File::create(save_dir)?;
		serde_json::to_writer_pretty(file, self)?;
		Ok(())
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_load_from_csv() {
		let test_csv_path = dev::env::DATA_DIR.to_path_buf().join("2025-04-25T0045_评分-AIB110002.13_Python程序设计.csv");
		let class = Class::load_from_csv(test_csv_path, None, None, true);
		assert!(class.is_ok());
		let class = class.unwrap();
		assert_eq!(class.id, "AIB110002.13");
		let class_dir = dev::env::DATA_DIR.to_path_buf().join("class.json");
		class.save(&class_dir).expect("TODO: panic message");
		println!("{}", class);
	}
}
