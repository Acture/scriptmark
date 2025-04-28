use common::defines::testresult::TestResult;
use common::defines::testsuite::TestSuite;
use scraper::Html;
use std::path::Path;
use std::sync::LazyLock;

type InputType = String;
type OutputType = String;
const SIMPLE_HTML_TEST_CASES: [(&str, &str); 21] = [
	// 基础标签测试
	("<p>段落</p>", "段落"),
	("<h1>标题</h1>", "标题"),
	("<div>块元素</div>", "块元素"),
	("<span>行内元素</span>", "行内元素"),

	// 简单文本和标签混合
	("文本和<b>粗体</b>", "文本和粗体"),
	("前缀<i>斜体</i>后缀", "前缀斜体后缀"),
	("这是<u>下划线</u>文本", "这是下划线文本"),

	// 单标签元素
	("换行<br>之后", "换行之后"),
	("换行<br/>之后", "换行之后"),
	("水平线<hr>之后", "水平线之后"),

	// 基本嵌套标签
	("<div><p>嵌套段落</p></div>", "嵌套段落"),
	("<p>外部<span>内部</span>文本</p>", "外部内部文本"),
	("<div><span>span</span>在<p>p</p>中</div>", "span在p中"),

	// 带属性的简单标签
	("<p class='test'>带类的段落</p>", "带类的段落"),
	("<div id='main'>带ID的区块</div>", "带ID的区块"),
	("<a href='https://example.com'>链接</a>", "链接"),

	// 简单列表
	("<ul><li>项目1</li></ul>", "项目1"),
	("<ol><li>第一</li><li>第二</li></ol>", "第一第二"),

	// 常见HTML元素组合
	("<article><h2>小标题</h2><p>内容</p></article>", "小标题内容"),
	("<section><h3>节标题</h3><p>节内容</p></section>", "节标题节内容"),

	// 特殊情况
	("没有标签纯文本", "没有标签纯文本"),
];

fn strip_html_tags(html: &InputType) -> Result<OutputType, String> {
	let document = Html::parse_document(html);
	Ok(document.root_element().text().collect::<Vec<_>>().join(""))
}

pub static STRIP_HTML_TAG_TESTSUITE: LazyLock<TestSuite<InputType, OutputType>> = LazyLock::new(|| {
	TestSuite {
		name: String::from("strip_html_tags"),
		inputs: SIMPLE_HTML_TEST_CASES.iter().map(|(html, _)| html.to_string()).collect(),
		answers: SIMPLE_HTML_TEST_CASES.iter().map(|(_, answer)| Ok(answer.to_string())).collect(),
		check_fn,
		answer_fn: strip_html_tags,
		run_file_fn: answer_file_fn,
	}
});

fn check_fn(expected: &Result<OutputType, String>, answer: &Result<OutputType, String>) -> Result<TestResult, String> {
	let expected = expected.as_ref().map_err(|e| e.to_string())?;
	let answer = answer.as_ref().map_err(|e| e.to_string())?.trim().to_string();

	Ok(TestResult::builder()
		.passed(answer.ends_with(expected))
		.messages(vec![format!("Expected: {}, Actual: {}", expected, answer)])
		.build())
}

fn answer_file_fn(path: &Path, input: &InputType) -> Result<OutputType, String> {
	let mut input_with_t = input.clone();
	input_with_t.push('\n');
	match code_runner::python::run_from_file::<String>(path, Some(input_with_t), &["re".to_string()])
	{
		Ok((output, _trace)) => Ok(output),
		Err(e) => Err(e.to_string()),
	}
}


mod tests {
	use super::*;

	#[test]
	fn test_simple_html_tags() {
		for (html, expected) in SIMPLE_HTML_TEST_CASES.iter() {
			assert_eq!(strip_html_tags(&html.to_string()), Ok(expected.to_string()));
		}
	}
}