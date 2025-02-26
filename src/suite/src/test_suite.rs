use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct TestResult {
	#[builder(default = false)]
	pub passed: bool,
	#[builder(default = None)]
	pub infos: Option<HashMap<String, Option<Vec<String>>>>,
}

pub trait TestSuiteTrait {
	fn run(&self, path: &dyn AsRef<PathBuf>) -> Box<dyn Any>;
	fn judge(&self, result: &dyn Any, expected: &dyn Any) -> TestResult;
}

pub struct TestSuite {
	data: Box<dyn Any>,
	answer: Box<dyn Any>,
	runner: Box<dyn Fn(&dyn AsRef<PathBuf>) -> Box<dyn Any>>,
	judge: Box<dyn Fn(&dyn Any, &dyn Any) -> TestResult>,
}

impl TestSuite {
	pub fn new(
		data: Box<dyn Any>,
		answer: Box<dyn Any>,
		runner: Box<dyn Fn(&dyn AsRef<PathBuf>) -> Box<dyn Any>>,
		judge: Box<dyn Fn(&dyn Any, &dyn Any) -> TestResult>,
	) -> Self {
		Self {
			data,
			answer,
			runner,
			judge,
		}
	}
}

impl TestSuiteTrait for TestSuite {
	fn run(&self, path: &dyn AsRef<PathBuf>) -> Box<dyn Any> {
		(self.runner)(path)
	}

	fn judge(&self, result: &dyn Any, expected: &dyn Any) -> TestResult {
		(self.judge)(result, expected)
	}
}
