use common::defines::testresult::TestResult;
use common::defines::testsuite::TestSuite;
use regex::Regex;
use std::iter::Iterator;
use std::path::Path;
use std::sync::LazyLock;

// 有效的电子邮件地址


type InputType = String;
type OutputType = String;

const TEST_EMAILS: [(&str, bool); 14] = [
	// (邮箱地址, 是否有效)

	// 有效的电子邮件地址
	("simple@example.com", true),                 // 基本格式
	("x@example.com", true),                      // 单字符本地部分
	("cs@fudan.edu.cn", true),
	("cs1@fudan.edu.cn", true),
	("cs_2@fudan.edu.cn", true),
	("cs234@fudan.edu.cn", true),
	// 无效的电子邮件地址
	("cs&@fudan.edu.cn", false),
	("", false),                                  // 空字符串
	("abc", false),                               // 没有@符号
	("@example.com", false),                      // 没有本地部分
	("abc@", false),                              // 没有域名部分
	("a@b", false),                               // 域名太短/无顶级域名
	("abc@example.com def", false),               // 域名有空格
	("john@example.com,jane@example.com", false), // 多个邮箱
];


pub static VALID_EMAIL_TESTSUITE: LazyLock<TestSuite<InputType, OutputType>> = LazyLock::new(|| {
	TestSuite {
		name: String::from("valid_email"),
		inputs: TEST_EMAILS.iter().map(|(email, _)| email.to_string()).collect(),
		answers: TEST_EMAILS.iter().map(|(_, answer)| Ok(answer.to_string())).collect(),
		check_fn,
		answer_fn: is_valid_email,
		run_file_fn: answer_file_fn,
	}
});

fn answer_file_fn(path: &Path, input: &InputType) -> Result<OutputType, String> {
	let mut input_with_t = input.clone();
	input_with_t.push('\n');
	match code_runner::python::run_from_file::<String>(&path, Some(input_with_t), &["re".to_string()], false, None)
	{
		Ok((output, _trace)) => Ok(output),
		Err(e) => Err(e.to_string()),
	}
}

fn check_fn(expected: &Result<OutputType, String>, actual: &Result<OutputType, String>) -> Result<TestResult, String> {
	let bool_expected = expected.as_ref()?.parse::<bool>().map_err(|e| e.to_string())?;
	let actual = actual.as_ref()?.trim();

	Ok(TestResult::builder()
		.passed(
			match bool_expected {
				true => actual.ends_with("True") || actual.ends_with("true"),
				false => actual.ends_with("False") || actual.ends_with("false"),
			}
		)
		.messages(Vec::from([format!("expected: {:?}, actual: {:?}", expected, actual)]))
		.build())
}

fn is_valid_email(email: &InputType) -> Result<OutputType, String> {
	let re = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
		.map_err(|e| format!("invalid regex: {}", e))?;
	// 检查邮箱字符串是否匹配正则表达式
	if !re.is_match(email) {
		return Ok(false.to_string());
	}

	// 附加检查: 确保没有连续的点
	if email.contains("..") {
		return Ok(false.to_string());
	}

	// 附加检查: 确保@前后有内容
	let parts: Vec<&str> = email.split('@').collect();
	if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
		return Ok(false.to_string());
	}

	Ok(true.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn test_is_effective_email() {
		for (email, expected) in TEST_EMAILS {
			assert_eq!(is_valid_email(&email.to_string()), Ok(expected.to_string()), "email: {}", email);
		}
	}
}