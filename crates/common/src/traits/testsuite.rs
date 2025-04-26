use crate::defines::testresult::TestResult;
use dyn_clone::DynClone;
use serde_json::Value;

pub trait TestSuite {
	type Input;
	type Output;

	fn name(&self) -> &str;

	fn input(&self) -> Self::Input;

	fn answer(&self, input: &Self::Input) -> Self::Output;

	fn run(&self, input: &Self::Input) -> Result<Self::Output, String>;

	fn judge(&self, expected: &Self::Output, actual: &Self::Output) -> TestResult;
}

pub trait DynTestSuite: DynClone + std::fmt::Debug {
	fn name(&self) -> &str;

	fn input(&self) -> Value;

	fn answer(&self, input: &Value) -> Value;

	fn run(&self, input: &Value) -> Result<Value, String>;

	fn judge(&self, expected: &Value, actual: &Value) -> TestResult;
}