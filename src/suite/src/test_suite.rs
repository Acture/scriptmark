use std::collections::HashMap;
use typed_builder::TypedBuilder;
use std::path::PathBuf;

#[derive(Debug, TypedBuilder)]
pub struct TestResult {
	#[builder(default = false)]
	pub passed: bool,
	#[builder(default = None)]
	pub infos: Option<HashMap<String, Option<Vec<String>>>>,
}
pub struct TestSuite<I, O, P>
where
	P: AsRef<PathBuf>,
{
	data: I,
	answer: O,
	runner: Box<dyn Fn(&P) -> O>,
	judge: Box<dyn Fn(&O, &O) -> TestResult>,
}

impl<I, R, P: AsRef<PathBuf>> TestSuite<I, R, P> {
	pub fn new(
		data: I,
		answer: R,
		runner: Box<dyn Fn(&P) -> R>,
		judge: Box<dyn Fn(&R, &R) -> TestResult>,
	) -> Self {
		Self {
			data,
			answer,
			runner,
			judge,
		}
	}

	pub fn run(&self, path: &P) -> TestResult {
		(self.judge)(&(self.runner)(path), &self.answer)
	}
}
