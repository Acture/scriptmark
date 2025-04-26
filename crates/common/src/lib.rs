use crate::defines::assignment::Assignment;
use crate::defines::class::Class;
use crate::defines::student::Student;
use crate::defines::submission::Submission;
use std::cell::RefCell;
use std::fs::File;
use std::path::Path;
use std::rc::Rc;

pub mod defines;
pub mod traits;
pub mod utils;
mod types;
mod macros;


pub fn parse_csv_name(path: &Path) -> Result<(String, String, String), Box<dyn std::error::Error>> {
	let file_stem = path.file_stem()
		.ok_or("Missing file stem in path")?
		.to_str()
		.ok_or("File stem is not valid UTF-8")?;

	let parts: Vec<&str> = file_stem.splitn(4, |c| ['-', '_'].contains(&c)).collect();

	let (time, _, class_id, class_name) = match parts.as_slice() {
		[time, _grade, class_id, class_name] => (time.to_string(), _grade.to_string(), class_id.to_string(), class_name.to_string()),
		_ => return Err("Invalid filename format".into()),
	};


	Ok((time, class_id, class_name))
}

pub fn parse_from_csv(csv_path: &Path) -> Result<Class, Box<dyn std::error::Error>> {
	let (_, class_id, class_name) = parse_csv_name(csv_path)?;

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
	let assignments: Vec<Rc<RefCell<Assignment>>> = score_range.clone().map(|i| {
		let name = headers.get(i).unwrap().to_string();
		let score = points_possible_record.get(i).unwrap().parse::<f64>().unwrap();
		Rc::new(RefCell::new(Assignment::builder()
			.name(name)
			.points_possible(score)
			.build()
		))
	}).collect();
	let mut students: Vec<Rc<RefCell<Student>>> = Vec::new();

	for result in rdr.records() {
		let record = result?;
		let name = record.get(student_name_index).unwrap().to_string();
		if name == "测验学生" {
			continue;
		}
		let student = rc_ref!(Student::builder()
			.id(record.get(student_id_index).unwrap().to_string())
			.name(record.get(student_name_index).unwrap().to_string())
			.sis_login_id(record.get(student_sis_login_id).unwrap().to_string())
			.build());
		let weak_student = Rc::downgrade(&student);
		let submissions: Vec<Rc<RefCell<Submission>>> = score_range
			.clone()
			.zip(assignments.iter())
			.filter_map(|(i, assignment)| {
				let score = record.get(i)?.parse::<f64>().ok()?;
				Some(
					rc_ref!(Submission::builder().student(weak_student.clone()).score(score).build())
				)
			})
			.collect();

		student.borrow_mut().submissions = submissions;
		students.push(
			student
		);
	};


	Ok(
		Class::builder()
			.id(class_id)
			.name(class_name)
			.students(students)
			.assignments(assignments)
			.build()
	)
}


pub fn save(classes: &[Class]) {
	for class in classes {
		let serializable = class.to_serializable();
		let json = serde_json::to_string_pretty(&serializable).unwrap();
		println!("{}", json);
	}
}

mod tests {
	use super::*;
	use dev::env::DATA_DIR;
	use std::fs;
	use std::path::PathBuf;

	fn csv_paths() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
		Ok(fs::read_dir(DATA_DIR.to_path_buf())?
			.filter_map(|e| {
				match e {
					Ok(e) => match e.path().extension() {
						Some(ext) if ext == "csv" => Some(e.path()),
						_ => None,
					},
					Err(_) => None,
				}
			})
			.collect())
	}

	fn parsed_classes() -> types::ResultWithStdErr<Vec<Class>> {
		Ok(csv_paths()?
			.iter()
			.filter_map(|p| parse_from_csv(p).ok())
			.collect())
	}

	#[test]
	fn test_parse_from_csv() -> types::ResultWithStdErr<()> {
		for class in parsed_classes()? {
			assert!(!class.students.is_empty(), "Parsed class has no students");
			assert!(!class.assignments.is_empty(), "Parsed class has no assignments");
		}

		Ok(())
	}

	#[test]
	fn test_save() -> types::ResultWithStdErr<()> {
		let classes = parsed_classes()?;
		save(&classes);

		Ok(())
	}
}