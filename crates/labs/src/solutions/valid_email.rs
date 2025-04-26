use regex::Regex;

// 有效的电子邮件地址
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
fn is_valid_email(email: &str) -> bool {
	let re = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
	// 检查邮箱字符串是否匹配正则表达式
	if !re.is_match(email) {
		return false;
	}

	// 附加检查: 确保没有连续的点
	if email.contains("..") {
		return false;
	}

	// 附加检查: 确保@前后有内容
	let parts: Vec<&str> = email.split('@').collect();
	if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
		return false;
	}

	true
}


#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn test_is_effective_email() {
	for (email, expected) in TEST_EMAILS {
		assert_eq!(is_valid_email(email), expected, "email: {}", email);
		}
	}
}