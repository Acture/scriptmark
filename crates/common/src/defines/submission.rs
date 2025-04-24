use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder, Serialize, Deserialize, Clone)]
pub struct Submission {
	pub score: Option<f64>,
	pub submission_path: PathBuf,
	pub cached_hash: Option<u64>,
}

impl Submission {
	pub fn update_hash(&mut self) {
		if !self.submission_path.exists() {
			panic!("Submission path does not exist: {}", self.submission_path.display());
		}


		let contents = std::fs::read(self.submission_path.as_path()).unwrap();
		let mut hasher = DefaultHasher::new();
		contents.hash(&mut hasher);
		let hash = hasher.finish();
		self.cached_hash = Some(hash);
	}
}