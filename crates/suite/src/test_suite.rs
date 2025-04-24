use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use typed_builder::TypedBuilder;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum AdditionalStatus {
	None,
	Partial,
	Full,
}

impl fmt::Display for AdditionalStatus {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let s = match self {
			AdditionalStatus::None => "None",
			AdditionalStatus::Partial => "Partial",
			AdditionalStatus::Full => "Full",
		};
		write!(f, "{}", s)
	}
}

impl FromStr for AdditionalStatus {
	type Err = String; // 可以使用自定义错误类型

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"None" => Ok(AdditionalStatus::None),
			"Partial" => Ok(AdditionalStatus::Partial),
			"Full" => Ok(AdditionalStatus::Full),
			_ => Err(format!("Invalid value for AdditionalStatus: {}", s)),
		}
	}
}

#[derive(Debug, TypedBuilder, PartialEq, Clone)]
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
	fn get_input_type(&self) -> &'static str;
	fn get_output_type(&self) -> &'static str;
	fn get_data_any(&self) -> Box<dyn Any>;
	fn get_answer_any(&self) -> Box<dyn Any>;
	fn run_any(&self, path: &Path) -> Box<dyn Any>;
	fn judge_any(&self, result: &dyn Any, expected: &dyn Any) -> Vec<TestResult>;
}

impl<T> TestSuiteObject for T
where
	T: TestSuiteTrait + Send + Sync + 'static, // 约束 T 必须实现 TestSuiteTrait
	T::Input: 'static,                         // 确保 Input 可以转换为 Box<dyn Any>
	T::Output: 'static,                        // 确保 Output 可以转换为 Box<dyn Any>
{
	fn get_input_type(&self) -> &'static str {
		std::any::type_name::<T::Input>()
	}

	fn get_output_type(&self) -> &'static str {
		std::any::type_name::<T::Output>()
	}
	fn get_data_any(&self) -> Box<dyn Any> {
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
		::lazy_static::lazy_static! {
			static ref INPUTS: $input_type = $input_init;
			static ref ANSWERS: $answer_type = $answer_init;
			pub static ref $suite_name: ::suite::test_suite::TestSuite<
				$input_type,
				$answer_type,
				for<'a> fn(&'a ::std::path::Path) -> $answer_type,
				for<'a, 'b> fn(&'a $answer_type, &'b $answer_type) -> Vec<::suite::test_suite::TestResult>,
			> = ::suite::test_suite::TestSuite::builder()
				.inputs($input_clone(&INPUTS))
				.answers($answer_clone(&ANSWERS))
				.runner($runner as fn(&::std::path::Path) -> $answer_type)
				.judge($judge as fn(&$answer_type, &$answer_type) -> Vec<::suite::test_suite::TestResult>)
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_additional_status() {
		let status = AdditionalStatus::None;
		assert_eq!(status.to_string(), "None");
		match AdditionalStatus::from_str("None") {
			Ok(AdditionalStatus::None) => {
				assert_eq!(status, AdditionalStatus::None);
			}
			_ => panic!("Expected None"),
		}
	}

	#[test]
	fn test_test_result() {
		let result = TestResult::builder().passed(true).build();
		assert_eq!(result.passed, true);
		assert_eq!(result.infos, None);
		assert_eq!(result.additional_status, None);
		assert_eq!(result.additional_infos, None);
	}

	#[test]
	fn test_test_suite() {
		let suite = TestSuite::new(
			42,
			"Hello, world!".to_string(),
			|_| "Hello, world!".to_string(),
			|_, _| vec![],
		);
		let data = suite.get_data();
		let answer = suite.get_answer();

		assert_eq!(data, 42);
		assert_eq!(answer, "Hello, world!".to_string());
	}

	#[test]
	fn test_test_trait() {
		let suite = TestSuite::new(
			42,
			"Hello, world!".to_string(),
			|_| "Hello, world!".to_string(),
			|_, _| vec![],
		);
		let boxed =
			Box::new(suite.clone()) as Box<dyn TestSuiteTrait<Input=i32, Output=String>>;
		let data = suite.get_data();
		let answer = suite.get_answer();
		let boxed_data = boxed.get_data();
		let boxed_answer = boxed.get_answer();

		assert_eq!(data, boxed_data);
		assert_eq!(answer, boxed_answer);
	}

	#[test]
	fn test_test_suite_object() {
		let suite = TestSuite::new(
			42,
			"Hello, world!".to_string(),
			|_| "Hello, world!".to_string(),
			|_, _| vec![],
		);

		let trait_object =
			Box::new(suite.clone()) as Box<dyn TestSuiteTrait<Input=i32, Output=String>>;

		let object = Box::new(suite.clone()) as Box<dyn TestSuiteObject>;

		let data = suite.get_data();
		let trait_data = trait_object.get_data();
		let _object_data = object.get_data_any();
		let fake_test = Box::new(suite.get_answer()) as Box<dyn Any>;
		let answer = Box::new(suite.get_answer()) as Box<dyn Any>;

		assert_eq!(data, trait_data);

		println!("{:?}", object.get_input_type());
		println!("{:?}", object.get_output_type());

		let judge_res = object.judge_any(answer.as_ref(), fake_test.as_ref());
		println!("{:?}", judge_res);
	}
}
