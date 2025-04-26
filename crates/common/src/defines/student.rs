use crate::defines::submission::Submission;
use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::defines::class::Class;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct Student {
	pub name: String,
	pub id: String,
	pub sis_login_id: String,

	#[serde(skip)]
	#[builder(default)]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub submissions: Vec<Rc<RefCell<Submission>>>,
	#[serde(skip)]
	#[builder(default)]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub belong_to_class: Weak<RefCell<Class>>,
}

impl Student {
	pub fn to_serializable(&self) -> SerializableStudent {
		SerializableStudent {
			name: self.name.clone(),
			id: self.id.clone(),
			sis_login_id: self.sis_login_id.clone(),
			belong_to_class_id: match self.belong_to_class.upgrade() {
				Some(class) => Some(class.borrow().id.clone()),
				None => None,
			},
		}
	}

	pub fn from_serializable(serializable: SerializableStudent, classes: &[Rc<RefCell<Class>>]) -> Self {
		let belong_to_class = match serializable.belong_to_class_id {
			None => None,
			Some(id) => classes.iter().find(|class| class.borrow().id.clone() == id),
		};
		Self {
			name: serializable.name,
			id: serializable.id,
			sis_login_id: serializable.sis_login_id,
			submissions: vec![],
			belong_to_class: match belong_to_class {
				Some(class) => Rc::downgrade(class),
				None => Weak::new(),
			},
		}
	}
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct SerializableStudent {
	pub name: String,
	pub id: String,
	pub sis_login_id: String,

	pub belong_to_class_id: Option<String>,
}


#[cfg(test)]
mod tests {}
