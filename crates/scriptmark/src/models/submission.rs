use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A single file belonging to a student's submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudentFile {
	pub path: PathBuf,
	pub language: String,
}

/// All student submissions for a grading session, grouped by student ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionSet {
	/// student_id -> list of files
	pub by_student: HashMap<String, Vec<StudentFile>>,
}

impl SubmissionSet {
	pub fn student_ids(&self) -> Vec<String> {
		let mut ids: Vec<String> = self.by_student.keys().cloned().collect();
		ids.sort();
		ids
	}

	pub fn student_count(&self) -> usize {
		self.by_student.len()
	}

	pub fn languages(&self) -> Vec<String> {
		let mut langs: Vec<String> = self
			.by_student
			.values()
			.flatten()
			.map(|f| f.language.clone())
			.collect::<std::collections::HashSet<_>>()
			.into_iter()
			.collect();
		langs.sort();
		langs
	}
}
