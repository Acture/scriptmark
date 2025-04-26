use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Default, Clone, PartialEq, Eq)]
pub struct TestResult {
	pub passed: bool,
	pub message: Vec<String>,
}