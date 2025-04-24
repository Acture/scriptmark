use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder, Serialize, Deserialize, Clone)]
pub struct Submission {
	pub score: Option<f64>,
	pub submission_path: String,
}