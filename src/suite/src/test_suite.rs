use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use typed_builder::TypedBuilder;
use util;

#[derive(Debug)]
pub enum AdditionalStatus {
	None,
	Partial,
	Full,
}

#[derive(Debug, TypedBuilder)]
pub struct TestResult {
	#[builder(default = false)]
	pub passed: bool,
	#[builder(default = None)]
	pub infos: Option<HashMap<String, Vec<String>>>,
	#[builder(default = None)]
	pub additional_status: Option<AdditionalStatus>,
	#[builder(default = None)]
	pub additional_infos: Option<HashMap<String, Vec<String>>>,
}

pub trait TestSuiteTrait {
	fn get_data(&self) -> Box<dyn Any>;
	fn get_answer(&self) -> Vec<String>;
	fn run(&self, path: &Path) -> Vec<String>;
	fn judge(&self, result: &[String], expected: &[String]) -> Vec<TestResult>;
}
pub struct TestSuite<I, R, J>
where
	I: Any,
	R: Fn(&Path) -> Vec<String>,
	J: Fn(&[String], &[String]) -> Vec<TestResult>,
{
	data: I,
	answer: Vec<String>,
	runner: R,
	judge: J,
}

impl<I, F, J> TestSuite<I, F, J>
where
	I: 'static + Clone,
	F: 'static + Fn(&Path) -> Vec<String>,
	J: 'static + Fn(&[String], &[String]) -> Vec<TestResult>,
{
	pub fn new(data: I, answer: Vec<String>, runner: F, judge: J) -> Self {
		Self {
			data,
			answer,
			runner,
			judge,
		}
	}
}

impl<
		I: 'static + Clone,
		R: 'static + Fn(&Path) -> Vec<String>,
		J: 'static + Fn(&[String], &[String]) -> Vec<TestResult>,
	> TestSuiteTrait for TestSuite<I, R, J>
{
	fn get_data(&self) -> Box<dyn Any> {
		Box::new(self.data.clone())
	}

	fn get_answer(&self) -> Vec<String> {
		self.answer.clone()
	}

	fn run(&self, path: &Path) -> Vec<String> {
		let path_ref = path.as_ref();
        (self.runner)(path_ref)
	}

	fn judge(&self, result: &[String], expected: &[String]) -> Vec<TestResult> {
		(self.judge)(result, expected)
	}
}
