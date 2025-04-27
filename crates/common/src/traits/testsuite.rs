use crate::defines::testresult::TestResult;
use dyn_clone::DynClone;
use serde_json::Value;
use std::fmt::Debug;
use std::path::Path;

pub trait DynTestSuite: DynClone + Debug + Send + Sync {
	fn get_name(&self) -> &str;

	fn get_inputs(&self) -> Vec<Value>;

	fn get_answer(&self) -> Vec<Result<Value, String>>;

	fn run_answer_fn(&self, inputs: &[Value]) -> Vec<Result<Value, String>>;
	fn run_file(&self, path: &Path, inputs: &[Value]) -> Vec<Result<Value, String>>;
	fn judge(&self, expected_values: &[Result<Value, String>], actual_values: &[Result<Value, String>]) -> Vec<Result<TestResult, String>>;

	fn pipelined(&self, path: &Path) -> Vec<Result<TestResult, String>>;
}