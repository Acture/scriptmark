use std::path;
use std::path::Path;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
struct Class {
	name: String,
	path: path::PathBuf,
}

impl Class {
	fn new(name: String, path: path::PathBuf) -> Self {
		Self { name, path }
	}

	fn load_class<P: AsRef<Path>>(path: P) -> Vec<Class> {
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
}
