use crate::defines::task::Task;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::hash::Hash;
use typed_builder::TypedBuilder;


#[derive(TypedBuilder, Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
	pub name: String,
	#[builder(default = vec!())]
	pub tasks: Vec<Task>,
	#[builder(default = 100.0)]
	pub points_possible: f64,
}

impl Display for Assignment {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{} - Points {} - Task Num {}", self.name, self.points_possible, self.tasks.len())
	}
}

impl PartialEq for Assignment {
	fn eq(&self, other: &Self) -> bool {
		self.name == other.name
	}
}

impl Eq for Assignment {}

impl Hash for Assignment {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.name.hash(state);
	}
}

