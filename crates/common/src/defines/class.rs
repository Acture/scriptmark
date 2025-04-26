use crate::defines::assignment::Assignment;
use crate::defines::student::Student;
use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Class {
	#[builder(setter(into))]
	pub id: String,
	#[builder(setter(into))]
	pub name: String,
	#[serde(skip)]
	#[builder(default)]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub students: Vec<Rc<RefCell<Student>>>,
	#[builder(default)]
	#[serde(skip)]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub assignments: Vec<Rc<RefCell<Assignment>>>,
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct SerializableClass {
	pub id: String,
	pub name: String,
}



impl Class {
	pub fn to_serializable(&self) -> SerializableClass {
		SerializableClass {
			id: self.id.clone(),
			name: self.name.clone(),
		}
	}

	pub fn from_serializable(serializable: SerializableClass) -> Self {
		Class {
			id: serializable.id,
			name: serializable.name,
			..Default::default()
		}
	}
}


#[cfg(test)]
mod tests {
}
