use crate::defines::student::Student;
use std::collections::HashMap;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct SubmissionRecord {
	#[builder(default = None)]
	pub student: Option<Student>,
	#[builder(default = None)]
	pub assignment: Option<String>,
	#[builder(default = None)]
	pub is_submitted: Option<bool>,

	#[builder(default = None)]
	pub correct_count: Option<usize>,
	#[builder(default = None)]
	pub total_count: Option<usize>,
	#[builder(default = None)]
	pub did_additional: Option<bool>,
	#[builder(default = None)]
	pub has_hash_collision: Option<Vec<String>>,
	#[builder(default = None)]
	pub errors: Option<HashMap<String, Option<Vec<String>>>>,
	#[builder(default = None)]
	pub messages: Option<HashMap<String, Option<Vec<String>>>>,
}
