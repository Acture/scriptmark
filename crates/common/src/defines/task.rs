use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Debug, Clone, Serialize, Deserialize)]
pub struct Task {
	pub name: String,
	pub testsuite: String,
	#[builder(default = 100.0)]
	pub score: f64,
}

impl Display for Task {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "Task[{} - {}]", self.name, self.score)
	}
}

impl PartialEq for Task {
	fn eq(&self, other: &Self) -> bool {
		self.name == other.name
	}
}

impl Eq for Task {}

impl Hash for Task {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.name.hash(state);
	}
}