use crate::defines::testresult::TestResult;
use dyn_clone::DynClone;
use serde_json::Value;

pub trait DynTestSuite: DynClone + std::fmt::Debug {
	fn get_name(&self) -> &str;

	fn get_inputs(&self) -> Vec<Value>;

	fn get_answer(&self, inputs: &[Value]) -> Vec<Value>;

	fn run(&self, inputs: &[Value]) -> Vec<Result<Value, String>>;

	fn judge(&self, expected_values: &[Value], actual_values: &[Value]) -> Vec<Result<TestResult, String>>;
}