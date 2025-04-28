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
pub mod types;
pub mod macros;
pub mod dump;

pub fn parse_csv_name(path: &Path) -> Result<(String, String, String), Box<dyn std::error::Error>> {
	let file_stem = path.file_stem()
		.ok_or("Missing file stem in path")?
		.to_str()
		.ok_or("File stem is not valid UTF-8")?;

	let parts: Vec<&str> = file_stem.splitn(3, '_').collect();

	let (time, class_id, class_name) = match parts.as_slice() {
		[time, _id_part, class_name] => (
			time.to_string(),
			_id_part.to_string().rsplitn(2, '-').next().expect("Invalid id part").to_string(),
			class_name.to_string()
		),
		_ => return Err("Invalid filename format".into()),
	};


	Ok((time, class_id, class_name))
}

pub fn parse_from_csv(csv_path: &Path) -> Result<Rc<RefCell<Class>>, Box<dyn std::error::Error>> {
	let (_, class_id, class_name) = parse_csv_name(csv_path)?;

	let class = rc_ref!(Class::builder()
		.id(class_id)
		.name(class_name)
		.build());

	let file = File::open(csv_path)?;
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
	let points_possible_record = rdr.records().next().ok_or("Missing points possible row")?.unwrap();
	if !points_possible_record.get(0).unwrap().contains("Points Possible") {
		return Err("Expected first row to be 'Points Possible' row".into());
	}
	let assignments: Vec<Rc<RefCell<Assignment>>> = score_range.clone().map(|i| {
		let name = headers.get(i).unwrap().to_string();
		let score = points_possible_record.get(i).unwrap().parse::<f64>().unwrap();
		Rc::new(RefCell::new(Assignment::builder()
			.name(name)
			.points_possible(score)
			.belong_to_class(Rc::downgrade(&class))
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
			.belong_to_class(Rc::downgrade(&class))
			.build());
		let submissions: Vec<Rc<RefCell<Submission>>> = score_range
			.clone()
			.zip(assignments.iter())
			.filter_map(|(i, assignment)| {
				let score = record.get(i)?.parse::<f64>().ok()?;
				Some(
					rc_ref!(
						Submission::builder()
						.belong_to_student(Rc::downgrade(&student))
						.belong_to_assignment(Rc::downgrade(assignment))
						.score(score).build()
					)
				)
			})
			.collect();

		student.borrow_mut().submissions = submissions;
		students.push(
			student
		);
	};

	class.borrow_mut().students = students;
	class.borrow_mut().assignments = assignments;


	Ok(
		class
	)
}


#[cfg(test)]
mod tests {
	use super::*;
use crate::dump::{load_dump, save_dump};
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


	fn parsed_classes() -> types::ResultWithStdErr<Vec<Rc<RefCell<Class>>>> {
		Ok(csv_paths()?
			.iter()
			.filter_map(|p| parse_from_csv(p).ok())
			.collect())
	}

	#[test]
	fn test_parse_csv_name() -> types::ResultWithStdErr<()> {
		let names: Vec<_> = csv_paths()?
			.iter()
			.filter_map(|p| parse_csv_name(p).ok())
			.collect();
		for name in names {
			println!("{:?}", name)
		}
		Ok(())
	}

	#[test]
	fn test_parse_from_csv() -> types::ResultWithStdErr<()> {
		for class in parsed_classes()? {
			println!("{}", class.borrow().name);
			assert!(!class.borrow().students.is_empty(), "Parsed class has no students");
			assert!(!class.borrow().assignments.is_empty(), "Parsed class has no assignments");
		}

		Ok(())
	}

	#[test]
	fn test_save() -> types::ResultWithStdErr<()> {
		let classes: Vec<_> = parsed_classes()?;

		let save_path = DATA_DIR.join("test_save.json");

		save_dump(&classes, &save_path).expect("TODO: panic message");

		println!("Saved to: {:?}", save_path.to_str().unwrap_or("invalid path"));

		assert!(save_path.exists());

		Ok(())
	}

	#[test]
	fn test_load() -> types::ResultWithStdErr<()> {
		let parsed_classes: Vec<_> = parsed_classes()?;
		let save_path = DATA_DIR.join("test_save.json");
		let loaded_classed = load_dump(&save_path)?;

		assert_eq!(parsed_classes, loaded_classed);

		Ok(())
	}
}