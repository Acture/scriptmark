use crate::defines::keyed::Keyed;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder, Clone, Serialize, Deserialize)]
pub struct Student {
	pub name: String,
	pub id: String,
	pub sis_login_id: String,

	#[builder(default)]
	pub submissions: HashMap<String, f64>,
}

impl Display for Student {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		writeln!(f, "{} ({}): ", self.name, self.sis_login_id)?;
		let sorted_submissions: Vec<_> = self.submissions.iter().sorted_by_key(
			|(assignment_key, _)| *assignment_key
		).collect();
		for (assignment_key, &score) in &sorted_submissions {
			writeln!(f, "\t - {} ({})", assignment_key, score)?;
		}
		Ok(())
	}
}

impl PartialEq for Student {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}

impl Eq for Student {}

impl Hash for Student {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state);
	}
}

impl Student {}

impl Keyed for Student {
	type Key = (String, String);

	fn key(&self) -> Self::Key {
		(self.id.clone(), self.name.clone())
	}
}
#[cfg(test)]
mod tests {}
