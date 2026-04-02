pub mod builtin;
pub mod python_checker;
pub mod rhai_checker;

use serde::{Deserialize, Serialize};

/// Input to a checker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckInput {
	/// What the student's code produced.
	pub result: serde_json::Value,
	/// What was expected (from the test spec).
	#[serde(default)]
	pub expected: serde_json::Value,
	/// Additional context for complex verifiers.
	#[serde(default)]
	pub context: serde_json::Value,
}

/// Output from a checker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckOutput {
	pub pass: bool,
	#[serde(default)]
	pub message: String,
}

/// Trait for all checkers (built-in and external).
pub trait Checker: Send + Sync {
	fn check(&self, input: &CheckInput) -> CheckOutput;
}
