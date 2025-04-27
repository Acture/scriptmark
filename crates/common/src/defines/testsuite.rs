use crate::defines::testresult::TestResult;
use crate::traits::testsuite::DynTestSuite;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;
use std::path::Path;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Debug, Clone)]
pub struct TestSuite<I, O> {
	#[builder(setter(into))]
	pub name: String,
	#[builder(setter(into))]
	pub inputs: Vec<I>,
	#[builder(setter(into))]
	pub answers: Vec<Result<O, String>>,

	#[builder(default = unimplemented_answer_fn)]
	pub answer_fn: fn(&I) -> Result<O, String>,
	#[builder(default = unimplemented_run_file_fn)]
	pub run_file_fn: fn(&Path, &I) -> Result<O, String>,
	#[builder(default = unimplemented_check_fn::<I, O>)]
	pub check_fn: fn(&Result<O, String>, &Result<O, String>) -> Result<TestResult, String>,
}

fn unimplemented_answer_fn<I, O>(_: &I) -> Result<O, String> {
	unimplemented!("answer_fn is not set")
}

fn unimplemented_run_file_fn<I, O>(_: &Path, _: &I) -> Result<O, String> {
	unimplemented!("run_file_fn is not set")
}

fn unimplemented_check_fn<I, O>(_: &Result<O, String>, _: &Result<O, String>) -> Result<TestResult, String> {
	unimplemented!("check_fn is not set")
}

impl<I, O> DynTestSuite for TestSuite<I, O>
where
	I: Serialize + for<'de> Deserialize<'de> + Clone + Debug + Send + Sync + 'static,
	O: Serialize + for<'de> Deserialize<'de> + Clone + Debug + Send + Sync + 'static,
{
	fn get_name(&self) -> &str {
		&self.name
	}
	fn get_inputs(&self) -> Vec<Value> {
		self.inputs
			.iter()
			.map(|i| serde_json::to_value(i).expect("Serialization failed"))
			.collect()
	}

	fn get_answer(&self) -> Vec<Result<Value, String>> {
		self.answers
			.iter()
			.map(|a| serde_json::to_value(a).map_err(|e| e.to_string()))
			.collect()
	}


	fn run_answer_fn(&self, inputs: &[Value]) -> Vec<Result<Value, String>> {
		inputs
			.iter()
			.map(|i|
				{
					let v = serde_json::from_value(i.clone()).expect("Deserialization failed");
					let raw_r = (self.answer_fn)(&v);
					Ok(serde_json::to_value(raw_r).expect("Serialization failed"))
				}
			)
			.collect()
	}

	fn run_file(&self, path: &Path, inputs: &[Value]) -> Vec<Result<Value, String>> {
		inputs
			.iter()
			.map(|i| {
				let v = serde_json::from_value(i.clone()).expect("Deserialization failed");
				let raw_r = (self.run_file_fn)(path, &v);
				Ok(serde_json::to_value(raw_r).expect("Serialization failed"))
			})
			.collect::<Vec<_>>()
	}

	fn judge(&self, expected_values: &[Result<Value, String>], actual: &[Result<Value, String>]) -> Vec<Result<TestResult, String>> {
		expected_values
			.iter()
			.enumerate()
			.map(|(i, e_v)| {
				let e_v = e_v.clone().expect("Deserialization failed");
				let a_v = actual.get(i).expect("Index out of bounds").clone().expect("Deserialization failed");
				(self.check_fn)(
					&serde_json::from_value(e_v.clone()).expect("Deserialization failed"),
					&serde_json::from_value(a_v.clone()).expect("Deserialization failed"),
				)
					.map_err(|e| e.to_string())
			})
			.collect()
	}

	fn pipelined(&self, path: &Path) -> Vec<Result<TestResult, String>> {
		let inputs = self.get_inputs();
		let expected_values = self.get_answer();
		let actual = self.run_file(path, &inputs);
		self.judge(&expected_values, &actual)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_suite_derive() {
		let _ = TestSuite::<i32, i32>::builder().name("test");
	}
}