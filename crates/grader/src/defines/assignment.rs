use crate::defines::keyed::{ArcKey, Keyed};
use crate::defines::task::Task;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use typed_builder::TypedBuilder;

type StudentName = String;


#[derive(TypedBuilder, Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
	pub name: String,
	#[builder(default = vec!())]
	pub tasks: Vec<Task>,
	#[builder(default = 100.0)]
	pub points_possible: f64,
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


impl Keyed for Assignment {
	type Key = String;
	fn key(&self) -> Self::Key {
		self.name.clone()
	}
}