use crate::defines::class::Class;
use crate::defines::task::Task;
use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Assignment {
	pub name: String,
	#[builder(default = 100.0)]
	#[derivative(Hash = "ignore")]
	pub points_possible: f64,

	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub tasks: Vec<Rc<RefCell<Task>>>,
	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub belong_to_class: Weak<RefCell<Class>>,
}

impl Assignment {
	pub fn to_serializable(&self) -> SerializableAssignment {
		SerializableAssignment {
			name: self.name.clone(),
			points_possible: self.points_possible,
			belong_to_class_name: self.belong_to_class.upgrade().map(|class| class.borrow().name.clone()),

		}
	}

	pub fn from_serializable(serializable: SerializableAssignment, classes: &[Rc<RefCell<Class>>]) -> Self {
		let belong_to_class = match serializable.belong_to_class_name {
			Some(name) => {
				classes.iter().find(|class| class.borrow().name == name)
			}
			None => None,
		};
		Self {
			name: serializable.name,
			points_possible: serializable.points_possible,
			tasks: vec![],
			belong_to_class: match belong_to_class {
				Some(class) => Rc::downgrade(class),
				None => Weak::new()
			},

		}
	}
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct SerializableAssignment {
	pub name: String,
	#[derivative(Hash = "ignore")]
	pub points_possible: f64,

	pub belong_to_class_name: Option<String>,
}


