use std::collections::HashMap;
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct Assignment {
	pub name: String,
	pub path: PathBuf,
}


impl Assignment {
	pub fn group_by_student<S: AsRef<str>>(&self, student_ids: &[S]) -> HashMap<String, Vec<PathBuf>> {
		let mut rec: HashMap<String, Vec<PathBuf>> = student_ids.iter()
			.map(|id| (id.as_ref().to_string(), vec![]))
			.collect();


		for entry in self.path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();

			if path.is_dir() {
				let id = path.file_stem().expect("file_stem failed").to_string_lossy().to_string();
				path.read_dir().expect("read_dir call failed").for_each(
					|entry| {
						let entry = entry.expect("entry failed");
						let path = entry.path();
						rec.entry(id.clone()).or_insert_with(Vec::new).push(path);
					}
				);
			} else if path.is_file() {
				let file_name = path.file_name().expect("file_name failed").to_string_lossy().to_string();
				let id = file_name.split('_').next().expect("split failed").to_string();
				rec.entry(id.clone()).or_insert_with(Vec::new).push(path);
			}
		}

		rec.iter().map(
			|(id, file_paths)| {
				let dir = self.path.join(id);
				if !dir.exists() {
					std::fs::create_dir(&dir).expect("create_dir failed");
				}
				let new_paths: Vec<PathBuf> = file_paths.iter().map(
					|file_path| {
						let new_path = dir.join(file_path.file_name().expect("file_name failed"));
						if !file_path.starts_with(&dir) {
							std::fs::rename(&file_path, &new_path).expect("rename failed");
						}
						new_path
					}
				).collect();
				(id.clone(), new_paths)
			}
		).collect()
	}
}


#[cfg(test)]
mod tests {
	use crate::class::Class;
	use crate::config::Config;
	use crate::student::Student;

	#[test]
	fn test_group_by_student() {
		let config = Config::builder().build();
		let test_class = &Class::load_class(&config.data_dir)[0];
		let students = Student::load_from_roster(test_class.roster_path());
		let test_assignment = &test_class.load_assignments()[0];
		let paths = test_assignment.group_by_student(students.iter().map(|s| &s.sis_login_id).collect());
		println!("{:?}", paths);
	}
}