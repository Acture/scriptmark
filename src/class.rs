use crate::assignment::Assignment;
use std::path;
use std::path::Path;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct Class {
	pub name: String,
	pub path: path::PathBuf,
}

impl Class {
	fn new(name: String, path: path::PathBuf) -> Self {
		Self { name, path }
	}

	pub fn load_class<P: AsRef<Path>>(path: P) -> Vec<Class> {
		let path = path.as_ref();
		let mut classes = Vec::new();
		for entry in path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let class = Class::new(
					path.file_stem()
						.expect("file_stem failed")
						.to_string_lossy()
						.to_string(),
					path,
				);
				classes.push(class);
			}
		}
		classes
	}

	pub fn roster_path(&self) -> path::PathBuf {
		self.path.join("roster.csv")
	}

	pub fn load_assignments(&self) -> Vec<Assignment> {
		let mut assignments = Vec::new();
		for entry in self.path.read_dir().expect("read_dir call failed") {
			let entry = entry.expect("entry failed");
			let path = entry.path();
			if path.is_dir() {
				let assignment = Assignment::builder()
					.name(path.file_stem().expect("file_stem failed").to_string_lossy().to_string())
					.path(path)
					.build();
				assignments.push(assignment);
			}
		}
		assignments
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
		let assignments = classes[0].load_assignments();
		println!("{:?}", assignments);
	}
}
