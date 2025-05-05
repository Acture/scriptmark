use common::defines::testresult::TestResult;
use common::defines::testsuite::TestSuite;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

type InputType = String;
type OutputType = String;

const TEST_INPUTS: [&str; 20] = [
	"abc",
	"ABC",
	"aBcDeF",
	"123",
	"hello, world!",
	"\nhello\n",
	"This is line 1.\nline two.",
	"",
	"  mixed CASE With SPACES  ",
	"___abc___",
	"中文测试abc",
	"a1b2c3!@#",
	"~!@#$%^&*()",
	"\tTab\tSeparated\twords",
	"camelCaseAnd_snake_case",
	"a\nb\nc",
	"multiple\n\n\nnewlines",
	"e=mc^2",
	"foo.bar.baz",
	"Python3.10 rocks!",
];

const EXPECTED_OUTPUTS: [&str; 20] = [
	"ABC",
	"ABC",
	"ABCDEF",
	"123",
	"HELLO, WORLD!",
	"\nHELLO\n",
	"THIS IS LINE 1.\nLINE TWO.",
	"",
	"  MIXED CASE WITH SPACES  ",
	"___ABC___",
	"中文测试ABC",
	"A1B2C3!@#",
	"~!@#$%^&*()",
	"\tTAB\tSEPARATED\tWORDS",
	"CAMELCASEAND_SNAKE_CASE",
	"A\nB\nC",
	"MULTIPLE\n\n\nNEWLINES",
	"E=MC^2",
	"FOO.BAR.BAZ",
	"PYTHON3.10 ROCKS!",
];

const IN_FILE_NAME: &str = "Lab9_in.txt";
const OUT_FILE_NAME: &str = "Lab9_out.txt";

fn lower_case_to_upper_case(input: &InputType) -> OutputType {
	input.to_uppercase()
}

fn answer_fn(input: &InputType) -> Result<OutputType, String> {
	Ok(lower_case_to_upper_case(input))
}


fn runner_fn(path: &Path, input: &InputType) -> Result<OutputType, String> {
	if !path.exists() {
		return Err(format!("File not found: {}", path.display()));
	}

	let in_file = PathBuf::from(IN_FILE_NAME);
	if in_file.exists() {
		fs::remove_file(&in_file).map_err(|e| format!("Failed to remove file: {}", e))?;
	}
	fs::write(&in_file, input).map_err(|e| format!("Failed to write to in file: {}", e))?;


	let (res, _) = code_runner::python::run_from_file::<String>(
		path,
		None,
		&[],
		false,
		Some(2),
	).map_err(|e| format!("Failed to run code: {}", e))?;

	let out_file = PathBuf::from(OUT_FILE_NAME);

	let out_content = std::fs::read_to_string(&out_file).map_err(|e| format!("Failed to read file: {}", e))?;
	fs::remove_file(&in_file).map_err(|e| format!("Failed to remove in file: {}", e))?;
	fs::remove_file(&out_file).map_err(|e| format!("Failed to remove out file: {}", e))?;
	Ok(out_content)
}

fn check_fn(input: &Result<OutputType, String>, expected: &Result<OutputType, String>) -> Result<TestResult, String> {
	let mut tr = TestResult::builder().passed(false).messages(vec![]).build();
	match (input, expected) {
		(Ok(input), Ok(expected)) => {
			if input == expected {
				tr.passed = true;
			} else {
				tr.passed = false;
				tr.messages.push(format!("Expected: {}, Got: {}", expected, input));
			}
		}
		_ => {
			tr.passed = false;
			tr.messages.push(format!("Expected: {:?}, Got: {:?}", expected, input));
		}
	};

	Ok(tr)
}

pub static LOWER_CASE_TO_UPPER_CASE_TESTSUITE: LazyLock<TestSuite<InputType, OutputType>> = LazyLock::new(|| {
	TestSuite {
		name: String::from("lower_case_to_upper_case"),
		inputs: TEST_INPUTS.iter().map(|s| s.to_string()).collect(),
		answers: EXPECTED_OUTPUTS.iter().map(|s| Ok(s.to_string())).collect(),
		answer_fn,
		run_file_fn: runner_fn,
		check_fn,
	}
});


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_lower_case_to_upper_case() {
		for (input, expected) in TEST_INPUTS.iter().zip(EXPECTED_OUTPUTS.iter()) {
			assert_eq!(lower_case_to_upper_case(&input.to_string()), expected.to_string());
		}
	}

	#[test]
	fn test_runner_fn() {
		let test_dir = dev::env::DATA_DIR.join("COMP110042.09/作业9");
		for entry in fs::read_dir(test_dir).unwrap() {
			let dir = entry.unwrap().path();
			if !dir.is_dir() {
				continue;
			}
			let test_file = dir.join("Lab9.py");
			if !test_file.exists() {
				continue;
			}
			println!("Running test file: {}", test_file.display());
			for (input, expected) in TEST_INPUTS.iter().zip(EXPECTED_OUTPUTS.iter()) {
				let res = runner_fn(&test_file, &input.to_string());
				if !res.is_ok() {
					println!("Failed to run test file: {}", test_file.display());
					println!("Input: {}", input);
					println!("Expected: {}", expected);
					println!("Got: {:?}", res);
				}
			}
		}
	}
}