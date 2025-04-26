use crate::defines::testresult::TestResult;
use crate::traits::testsuite::DynTestSuite;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
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
	pub answers: Vec<O>,

	pub answer_fn: fn(&I) -> Result<O, String>,
	pub check_fn: fn(&O, &O) -> Result<TestResult, String>,
}

fn panic_run_fn<I, O>(_: I) -> O {
	panic!("run_fn is not set")
}

fn panic_check_fn<I, O>(_: I, _: O) -> TestResult {
	panic!("check_fn is not set")
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

	fn get_answer(&self) -> Vec<Value> {
		self.answers
			.iter()
			.map(|a| serde_json::to_value(a).expect("Serialization failed"))
			.collect()
	}


	fn run_answer_fn(&self, inputs: &[Value]) -> Vec<Result<Value, String>> {
		inputs
			.iter()
			.map(|i| {
				let r = (self.answer_fn)(&serde_json::from_value(i.clone()).expect("Deserialization failed"));
				Ok(serde_json::to_value(r).expect("Serialization failed"))
			}
			)
			.collect::<Vec<_>>()
	}

	fn run_file(&self, path: &Path, inputs: &[Value]) -> Vec<Result<Value, String>> {
		todo!()
	}

	fn judge(&self, expected_values: &[Value], actual: &[Value]) -> Vec<Result<TestResult, String>> {
		expected_values
			.iter()
			.enumerate()
			.map(|(i, e_v)| {
				let a_v = actual.get(i).expect("Index out of bounds");
				(self.check_fn)(
					&serde_json::from_value(e_v.clone()).expect("Deserialization failed"),
					&serde_json::from_value(a_v.clone()).expect("Deserialization failed"),
				)
					.map_err(|e| e.to_string())
			})
			.collect()
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