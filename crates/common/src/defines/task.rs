use crate::defines::assignment::Assignment;
use crate::defines::submission::Submission;
use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fmt::Display;
use std::hash::Hash;
use std::rc::{Rc, Weak};
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Task {
	pub name: String,
	#[builder(default)]
	pub testsuite_name: Option<String>,
	#[builder(default = 100.0)]
	#[derivative(Hash = "ignore")]
	pub score: f64,

	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub submissions: Vec<Rc<RefCell<Submission>>>,
	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub assignment: Weak<RefCell<Assignment>>,
}

impl Task {
	pub fn to_serializable(&self) -> SerializableTask {
		SerializableTask {
			name: self.name.clone(),
			testsuite_name: self.testsuite_name.clone(),
			score: self.score,
			belongs_to_assignment_name: match self.assignment.upgrade() {
				Some(assignment) => {
					assignment.borrow().name.clone()
				}
				None => panic!("Assignment is not set for task"),
			},
		}
	}

	pub fn from_serializable(serializable: &SerializableTask, assignments: &[Rc<RefCell<Assignment>>]) -> Self {
		let belong_to_assignment = assignments.iter()
			.find(|assignment| assignment.borrow().name == serializable.belongs_to_assignment_name);
		Task {
			name: serializable.name.clone(),
			testsuite_name: serializable.testsuite_name.clone(),
			score: serializable.score,
			submissions: vec![],
			assignment: match belong_to_assignment {
				Some(assignment) => Rc::downgrade(assignment),
				None => Weak::new(),
			},
		}
	}
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct SerializableTask {
	pub name: String,
	pub testsuite_name: Option<String>,
	#[derivative(Hash = "ignore")]
	pub score: f64,

	pub belongs_to_assignment_name: String,
}
