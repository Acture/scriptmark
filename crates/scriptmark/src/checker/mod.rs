pub mod builtin;
pub mod python_checker;
pub mod rhai_checker;

use serde::{Deserialize, Serialize};

use crate::models::{Check, Fault};

/// Whether `check` itself compares against the expectation it is handed. When it does not,
/// a declared expectation is compared exactly as well, so every one the case declares holds.
/// Python and function checkers always receive it, and are taken to use it.
pub fn reads_expectation(check: &Check) -> bool {
	match check {
		Check::Builtin { .. } => check.needs_expectation(),
		Check::Rhai(expr) => rhai_checker::refers_to(expr, "expected"),
		Check::Python(_) | Check::Function(_) => true,
	}
}

/// Input to a checker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckInput {
	/// What the student's code produced.
	pub result: serde_json::Value,
	/// What was expected (from the test spec).
	#[serde(default)]
	pub expected: serde_json::Value,
	/// The rest of the observation: `{stdout, files}`.
	#[serde(default)]
	pub context: serde_json::Value,
}

/// A checker's verdict. Also the Python checker script's wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckOutput {
	pub pass: bool,
	#[serde(default)]
	pub message: String,
}

/// A checker that could not reach a verdict — which is not the same as "no".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError {
	/// Teacher when the checker itself misbehaved; environment when it could not run.
	pub fault: Fault,
	pub message: String,
}

impl CheckError {
	pub fn teacher(message: impl Into<String>) -> Self {
		Self {
			fault: Fault::Teacher,
			message: message.into(),
		}
	}

	pub fn environment(message: impl Into<String>) -> Self {
		Self {
			fault: Fault::Environment,
			message: message.into(),
		}
	}
}

/// Trait for all checkers (built-in and external).
pub trait Checker: Send + Sync {
	fn check(&self, input: &CheckInput) -> Result<CheckOutput, CheckError>;
}
