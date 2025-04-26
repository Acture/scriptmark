use crate::defines::testresult::TestResult;
use dyn_clone::DynClone;
use serde_json::Value;
use std::fmt::Debug;
use std::path::Path;

pub trait DynTestSuite: DynClone + Debug + Send + Sync {
	fn get_name(&self) -> &str;

	fn get_inputs(&self) -> Vec<Value>;

	fn get_answer(&self) -> Vec<Value>;

	fn run_answer_fn(&self, inputs: &[Value]) -> Vec<Result<Value, String>>;
	fn run_file(&self, path: &Path, inputs: &[Value]) -> Vec<Result<Value, String>>;

	fn judge(&self, expected_values: &[Value], actual_values: &[Value]) -> Vec<Result<TestResult, String>>;
}