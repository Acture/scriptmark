use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use typed_builder::TypedBuilder;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum AdditionalStatus {
	None,
	Partial,
	Full,
}

impl AdditionalStatus {
	pub fn to_string(&self) -> String {
		match self {
			AdditionalStatus::None => "None".to_string(),
			AdditionalStatus::Partial => "Partial".to_string(),
			AdditionalStatus::Full => "Full".to_string(),
		}
	}

	pub fn from_string(s: &str) -> Self {
		match s {
			"None" => AdditionalStatus::None,
			"Partial" => AdditionalStatus::Partial,
			"Full" => AdditionalStatus::Full,
			_ => panic!("Invalid AdditionalStatus: {}", s),
		}
	}
}

#[derive(Debug, TypedBuilder)]
pub struct TestResult {
	#[builder(default = false)]
	pub passed: bool,
	#[builder(default = None)]
	pub infos: Option<HashMap<String, String>>,
	#[builder(default = None)]
	pub additional_status: Option<AdditionalStatus>,
	#[builder(default = None)]
	pub additional_infos: Option<HashMap<String, String>>,
}

pub trait TestSuiteTrait {
	type Input: 'static;
	type Output: 'static;
	fn get_data(&self) -> Self::Input;
	fn get_answer(&self) -> Self::Output;
	fn run(&self, path: &Path) -> Self::Output;
	fn judge(&self, result: &Self::Output, expected: &Self::Output) -> Vec<TestResult>;
}

pub trait TestSuiteObject: Any + Send + Sync {
	fn get_data(&self) -> Box<dyn Any>;
	fn get_answer_any(&self) -> Box<dyn Any>;
	fn run_any(&self, path: &Path) -> Box<dyn Any>;
	fn judge_any(&self, result: &dyn Any, expected: &dyn Any) -> Vec<TestResult>;
}

impl<T> TestSuiteObject for T
where
	T: TestSuiteTrait + Send + Sync + 'static,  // 约束 T 必须实现 TestSuiteTrait
	T::Input: 'static,  // 确保 Input 可以转换为 Box<dyn Any>
	T::Output: 'static, // 确保 Output 可以转换为 Box<dyn Any>
{
	fn get_data(&self) -> Box<dyn Any> {
		let data = <T as TestSuiteTrait>::get_data(self);
		Box::new(data)
	}

	fn get_answer_any(&self) -> Box<dyn Any> {
		let answer = <T as TestSuiteTrait>::get_answer(self);
		Box::new(answer)
	}

	fn run_any(&self, path: &Path) -> Box<dyn Any> {
		let result = <T as TestSuiteTrait>::run(self, path);
		Box::new(result)
	}

	fn judge_any(&self, result: &dyn Any, expected: &dyn Any) -> Vec<TestResult> {
		let result = result
			.downcast_ref::<T::Output>()
			.expect("Failed to downcast result");
		let expected = expected
			.downcast_ref::<T::Output>()
			.expect("Failed to downcast expected");
		<T as TestSuiteTrait>::judge(self, result, expected)
	}
}

#[derive(Debug, TypedBuilder, Clone)]
pub struct TestSuite<I, O, FRunner, FJudge>
where
	FRunner: Fn(&Path) -> O,
	FJudge: Fn(&O, &O) -> Vec<TestResult>,
{
	inputs: I,
	answers: O,
	runner: FRunner, // 使用泛型存储闭包
	judge: FJudge,   // 使用泛型存储闭包
}
impl<I, O, FRunner, FJudge> TestSuite<I, O, FRunner, FJudge>
where
	FRunner: Fn(&Path) -> O,
	FJudge: Fn(&O, &O) -> Vec<TestResult>,
{
	pub fn new(inputs: I, answers: O, runner: FRunner, judge: FJudge) -> Self {
		Self {
			inputs,
			answers,
			runner,
			judge,
		}
	}
}

impl<I, O, FRunner, FJudge> TestSuiteTrait for TestSuite<I, O, FRunner, FJudge>
where
	I: Clone + 'static, // 确保 `I` 可以被 `.clone()` 调用
	O: Clone + 'static, // 确保 `O` 可以被 `.clone()` 调用
	FRunner: Fn(&Path) -> O,
	FJudge: Fn(&O, &O) -> Vec<TestResult>,
{
	type Input = I;
	type Output = O;

	fn get_data(&self) -> Self::Input {
		self.inputs.clone()
	}

	fn get_answer(&self) -> Self::Output {
		self.answers.clone()
	}

	fn run(&self, path: &Path) -> Self::Output {
		(self.runner)(path)
	}

	fn judge(&self, result: &Self::Output, expected: &Self::Output) -> Vec<TestResult> {
		(self.judge)(result, expected)
	}
}

#[macro_export]
macro_rules! define_test_suite {
	// 带 pub 的完整版本
	(
		pub name = $suite_name:ident,
		inputs = {
			type = $input_type:ty,
			init = $input_init:expr,
			clone = $input_clone:expr
		},
		answers = {
			type = $answer_type:ty,
			init = $answer_init:expr,
			clone = $answer_clone:expr
		},
		runner = $runner:expr,
		judge = $judge:expr
	) => {
		lazy_static! {
			static ref INPUTS: $input_type = $input_init;
			static ref ANSWERS: $answer_type = $answer_init;
			pub static ref $suite_name: TestSuite<
				$input_type,
				$answer_type,
				for<'a> fn(&'a Path) -> $answer_type,
				for<'a, 'b> fn(&'a $answer_type, &'b $answer_type) -> Vec<TestResult>,
			> = TestSuite::builder()
				.inputs($input_clone(&INPUTS))
				.answers($answer_clone(&ANSWERS))
				.runner($runner as fn(&Path) -> $answer_type)
				.judge($judge as fn(&$answer_type, &$answer_type) -> Vec<TestResult>)
				.build();
		}
	};

	// 带 pub 的简化版本（用于实现 Clone 的类型）
	(
		pub name = $suite_name:ident,
		inputs = {
			type = $input_type:ty,
			init = $input_init:expr
		},
		answers = {
			type = $answer_type:ty,
			init = $answer_init:expr
		},
		runner = $runner:expr,
		judge = $judge:expr
	) => {
		define_test_suite!(
			pub name = $suite_name,
			inputs = {
				type = $input_type,
				init = $input_init,
				clone = Clone::clone
			},
			answers = {
				type = $answer_type,
				init = $answer_init,
				clone = Clone::clone
			},
			runner = $runner,
			judge = $judge
		);
	};
}


