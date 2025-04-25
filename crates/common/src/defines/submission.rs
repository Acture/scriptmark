use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder, Serialize, Deserialize, Clone)]
pub struct Submission {
	#[builder(default)]
	pub score: Option<f64>,
	#[builder(default)]
	pub submission_path: Option<PathBuf>,
	#[builder(default)]
	pub cached_hash: Option<u64>,
}

impl Submission {
	pub fn update_hash(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		if let Some(path) = &self.submission_path {
			if !path.exists() {
				panic!("Submission path does not exist: {}", path.display());
			}
			let contents = std::fs::read(path)?;
			let mut hasher = DefaultHasher::new();
			contents.hash(&mut hasher);
			let hash = hasher.finish();
			self.cached_hash = Some(hash);
			Ok(())
		} else {
			Err("No submission path".into())
		}
	}
}