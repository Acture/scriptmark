use std::collections::HashMap;
use std::io::Write;

use scriptmark::models::*;
use scriptmark::runner::orchestrator;
use scriptmark::runner::python::PythonExecutor;

fn setup_test_dir() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();

	// Student "alice" — correct implementation
	std::fs::write(
		dir.path().join("alice_lab5.py"),
		r#"
def find_larger_number(a, b):
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        raise TypeError("Arguments must be numbers")
    return max(a, b)
"#,
	)
	.unwrap();

	// Student "bob" — buggy implementation (returns min instead of max)
	std::fs::write(
		dir.path().join("bob_lab5.py"),
		r#"
def find_larger_number(a, b):
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        raise TypeError("Arguments must be numbers")
    return min(a, b)
"#,
	)
	.unwrap();

	dir
}

fn test_spec() -> TestSpec {
	toml::from_str(
		r#"
[meta]
name = "find_larger_number"
file = "lab5.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "3 < 5"
args = [3, 5]
expect = 5

[[cases]]
name = "equal zero"
args = [0, 0]
expect = 0

[[cases]]
name = "negative"
args = [-3, -2]
expect = -2

[[cases]]
name = "invalid type"
args = ["a", 1]
expect_error = "TypeError"
"#,
	)
	.unwrap()
}

#[tokio::test]
async fn test_python_executor_correct_student() {
	let dir = setup_test_dir();
	let executor = PythonExecutor::new();

	let files = vec![StudentFile {
		path: dir.path().join("alice_lab5.py"),
		language: "python".to_string(),
	}];

	let spec = test_spec();

	// Test correct answers
	let result = executor
		.execute_case(&files, &spec, &spec.cases[0], 10)
		.await;
	assert_eq!(result.status, TestStatus::Passed, "case: 3 < 5");

	let result = executor
		.execute_case(&files, &spec, &spec.cases[1], 10)
		.await;
	assert_eq!(result.status, TestStatus::Passed, "case: equal zero");

	let result = executor
		.execute_case(&files, &spec, &spec.cases[2], 10)
		.await;
	assert_eq!(result.status, TestStatus::Passed, "case: negative");

	// Test expected error
	let result = executor
		.execute_case(&files, &spec, &spec.cases[3], 10)
		.await;
	assert_eq!(
		result.status,
		TestStatus::Passed,
		"case: invalid type should raise TypeError"
	);
}

#[tokio::test]
async fn test_python_executor_buggy_student() {
	let dir = setup_test_dir();
	let executor = PythonExecutor::new();

	let files = vec![StudentFile {
		path: dir.path().join("bob_lab5.py"),
		language: "python".to_string(),
	}];

	let spec = test_spec();

	// "3 < 5" — bob returns min(3,5)=3, expected 5 → FAIL
	let result = executor
		.execute_case(&files, &spec, &spec.cases[0], 10)
		.await;
	assert_eq!(
		result.status,
		TestStatus::Failed,
		"bob: 3<5 should fail (returns min)"
	);

	// "equal zero" — min(0,0)=0, expected 0 → PASS (edge case)
	let result = executor
		.execute_case(&files, &spec, &spec.cases[1], 10)
		.await;
	assert_eq!(
		result.status,
		TestStatus::Passed,
		"bob: equal zero still passes"
	);

	// TypeError case — bob has type checking, so this passes
	let result = executor
		.execute_case(&files, &spec, &spec.cases[3], 10)
		.await;
	assert_eq!(
		result.status,
		TestStatus::Passed,
		"bob: TypeError still passes"
	);
}

#[tokio::test]
async fn test_orchestrator_runs_all_students() {
	let dir = setup_test_dir();
	let executor = PythonExecutor::new();

	let submissions = SubmissionSet {
		by_student: HashMap::from([
			(
				"alice".to_string(),
				vec![StudentFile {
					path: dir.path().join("alice_lab5.py"),
					language: "python".to_string(),
				}],
			),
			(
				"bob".to_string(),
				vec![StudentFile {
					path: dir.path().join("bob_lab5.py"),
					language: "python".to_string(),
				}],
			),
		]),
	};

	let specs = vec![test_spec()];

	let results = orchestrator::run_all(&submissions, &specs, &executor, 10, Some(2)).await;

	assert_eq!(results.len(), 2);
	assert!(results.contains_key("alice"));
	assert!(results.contains_key("bob"));

	let alice = &results["alice"];
	assert_eq!(alice.status(), TestStatus::Passed);
	assert_eq!(alice.total_cases(), 4);
	assert_eq!(alice.total_passed(), 4);

	let bob = &results["bob"];
	assert_eq!(bob.status(), TestStatus::Failed);
	assert_eq!(bob.total_cases(), 4);
	// bob returns min instead of max: fails on "3<5" and "negative", passes "equal zero" and "TypeError"
	assert_eq!(bob.total_passed(), 2);
}

#[tokio::test]
async fn test_missing_file() {
	let _dir = setup_test_dir();
	let executor = PythonExecutor::new();

	// Empty file list — no matching file
	let files: Vec<StudentFile> = vec![];
	let spec = test_spec();

	let result = executor
		.execute_case(&files, &spec, &spec.cases[0], 10)
		.await;
	assert_eq!(result.status, TestStatus::Error);
	assert!(result.failure.unwrap().message.contains("No file matching"));
}

#[tokio::test]
async fn test_fixtures_and_refs() {
	let dir = tempfile::tempdir().unwrap();

	// Student code with two functions: make_pair returns [a,b], sum_pair sums a pair
	std::fs::write(
		dir.path().join("alice_math.py"),
		r#"
def make_pair(a, b):
    return [a, b]

def sum_pair(pair):
    return pair[0] + pair[1]
"#,
	)
	.unwrap();

	// TOML spec with setup + $ref
	let spec: TestSpec = toml::from_str(
		r#"
[meta]
name = "math_pipeline"
file = "math.py"
language = "python"

[[setup]]
id = "pair"
function = "make_pair"
args = [3, 7]

[[cases]]
name = "sum of setup pair"
function = "sum_pair"
args = ["$pair"]
expect = 10
"#,
	)
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_math.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;

	let alice = &results["alice"];
	assert_eq!(alice.total_cases(), 1);
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"sum_pair($pair) should pass with fixture pair=[3,7] → sum=10"
	);
}

#[tokio::test]
async fn test_vars_as_refs() {
	let dir = tempfile::tempdir().unwrap();

	std::fs::write(
		dir.path().join("alice_echo.py"),
		r#"
def echo(x):
    return x
"#,
	)
	.unwrap();

	let spec: TestSpec = toml::from_str(
		r#"
[meta]
name = "echo_test"
file = "echo.py"
language = "python"

[vars]
msg = "hello world"

[[cases]]
name = "echo var ref"
function = "echo"
args = ["$msg"]
expect = "hello world"
"#,
	)
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_echo.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];
	assert_eq!(alice.test_results[0].cases[0].status, TestStatus::Passed);
}

#[tokio::test]
async fn test_vars_injected_as_python_globals() {
	let dir = tempfile::tempdir().unwrap();

	// Student code accesses a global variable defined by the teacher
	std::fs::write(
		dir.path().join("alice_config.py"),
		r#"
def get_epsilon():
    return EPSILON  # This global is injected by vars
"#,
	)
	.unwrap();

	let spec: TestSpec = toml::from_str(
		r#"
[meta]
name = "config_test"
file = "config.py"
language = "python"

[vars]
EPSILON = 0.001

[[cases]]
name = "student reads injected global"
function = "get_epsilon"
args = []
expect = 0.001
"#,
	)
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_config.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"Student should be able to access EPSILON global injected by vars"
	);
}

#[tokio::test]
async fn test_parametrize_with_reference_oracle() {
	let dir = tempfile::tempdir().unwrap();

	// Student — correct max implementation
	std::fs::write(
		dir.path().join("alice_lab5.py"),
		"def find_max(a, b):\n    return max(a, b)\n",
	)
	.unwrap();

	// Teacher reference implementation (the oracle)
	let solutions_dir = dir.path().join("solutions");
	std::fs::create_dir(&solutions_dir).unwrap();
	std::fs::write(
		solutions_dir.join("lab5.py"),
		"def find_max(a, b):\n    return max(a, b)\n",
	)
	.unwrap();

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "parametrized_max"
file = "lab5.py"
function = "find_max"
language = "python"

[[cases]]
name = "random max"

[cases.parametrize]
count = 10
seed = 42

[cases.parametrize.args]
a = "int(-50, 50)"
b = "int(-50, 50)"

[cases.parametrize.oracle]
reference = "{}/solutions/lab5.py"
"#,
		dir.path().display()
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_lab5.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(alice.total_cases(), 10, "Should have 10 generated cases");
	assert_eq!(
		alice.total_passed(),
		10,
		"Correct implementation should pass all generated cases"
	);
}

#[tokio::test]
async fn test_parametrize_with_rhai_oracle() {
	let dir = tempfile::tempdir().unwrap();

	std::fs::write(
		dir.path().join("alice_lab5.py"),
		"def find_max(a, b):\n    return max(a, b)\n",
	)
	.unwrap();

	let spec: TestSpec = toml::from_str(
		r#"
[meta]
name = "rhai_oracle_max"
file = "lab5.py"
function = "find_max"
language = "python"

[[cases]]
name = "rhai oracle"

[cases.parametrize]
count = 5
seed = 123

[cases.parametrize.args]
a = "int(0, 100)"
b = "int(0, 100)"

[cases.parametrize.oracle]
rhai = "if a >= b { a } else { b }"
"#,
	)
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_lab5.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(alice.total_cases(), 5, "Should have 5 generated cases");
	assert_eq!(
		alice.total_passed(),
		5,
		"Correct max implementation should match Rhai oracle"
	);
}

#[tokio::test]
async fn test_setup_file_source() {
	let dir = tempfile::tempdir().unwrap();

	// Teacher generator script — outputs JSON to stdout
	std::fs::write(
		dir.path().join("gen_data.py"),
		r#"
import json
data = {"values": [10, 20, 30], "name": "test_data"}
print(json.dumps(data))
"#,
	)
	.unwrap();

	// Student code — reads the generated data
	std::fs::write(
		dir.path().join("alice_proc.py"),
		r#"
def sum_values(data):
    return sum(data["values"])
"#,
	)
	.unwrap();

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "file_source_test"
file = "proc.py"
function = "sum_values"
language = "python"

[[setup]]
id = "data"
file = "{}/gen_data.py"

[[cases]]
name = "sum generated values"
args = ["$data"]
expect = 60
"#,
		dir.path().display()
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_proc.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(alice.total_cases(), 1);
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"Should pass: teacher script generates data, student sums it correctly"
	);
}

// ============================================================
// Chain mode tests
// ============================================================

/// Helper to write a file and return its path as a string.
fn write_file(dir: &std::path::Path, name: &str, content: &str) -> String {
	let path = dir.join(name);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).unwrap();
	}
	let mut f = std::fs::File::create(&path).unwrap();
	f.write_all(content.as_bytes()).unwrap();
	path.to_string_lossy().to_string()
}

#[tokio::test]
async fn test_chain_basic_imports() {
	let dir = tempfile::tempdir().unwrap();

	// Teacher module: provides DATA
	let teacher_path = write_file(
		dir.path(),
		"teacher.py",
		r#"
class Dataset:
    def __init__(self, values):
        self.values = values

DATA = Dataset([10, 20, 30])
"#,
	);

	// Student: processes the teacher's DATA
	write_file(
		dir.path(),
		"alice_hw.py",
		r#"
def total(data):
    return sum(data.values)

def average(data):
    return sum(data.values) / len(data.values)
"#,
	);

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "chain_basic"
file = "hw.py"
language = "python"
imports = ["{teacher_path}"]

[[cases]]
name = "sum values"
function = "total"
args = ["$DATA"]
expect = 60

[[cases]]
name = "average values"
function = "average"
args = ["$DATA"]
expect = 20.0
"#,
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_hw.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(alice.total_cases(), 2);
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"total($DATA) should be 60"
	);
	assert_eq!(
		alice.test_results[0].cases[1].status,
		TestStatus::Passed,
		"average($DATA) should be 20.0"
	);
}

#[tokio::test]
async fn test_chain_decorator_checker() {
	let dir = tempfile::tempdir().unwrap();

	// Teacher module: DATA + @checker for validate
	let teacher_path = write_file(
		dir.path(),
		"teacher.py",
		r#"
DATA = list(range(10))

@checker
def check_subset(result, expected, DATA):
    """Auto-injected DATA from _ctx."""
    if not isinstance(result, list):
        return False, f"Expected list, got {type(result).__name__}"
    for item in result:
        if item not in DATA:
            return False, f"{item} not in DATA"
    return True, ""
"#,
	);

	// Student returns a subset
	write_file(
		dir.path(),
		"alice_hw.py",
		r#"
def subset(data, n):
    return data[:n]
"#,
	);

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "checker_test"
file = "hw.py"
language = "python"
imports = ["{teacher_path}"]

[[cases]]
name = "first 3 elements"
function = "subset"
args = ["$DATA", 3]
expect = [0, 1, 2]
"#,
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_hw.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(alice.total_cases(), 1);
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"@checker should validate subset against DATA"
	);
}

#[tokio::test]
async fn test_chain_decorator_checker_fails() {
	let dir = tempfile::tempdir().unwrap();

	let teacher_path = write_file(
		dir.path(),
		"teacher.py",
		r#"
DATA = [1, 2, 3]

@checker("get_items")
def validate_items(result, expected, DATA):
    for item in result:
        if item not in DATA:
            return False, f"{item} not in DATA"
    return True, ""
"#,
	);

	// Student returns items not in DATA
	write_file(
		dir.path(),
		"alice_hw.py",
		r#"
def get_items():
    return [1, 2, 99]
"#,
	);

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "checker_fail_test"
file = "hw.py"
language = "python"
imports = ["{teacher_path}"]

[[cases]]
name = "bad items"
function = "get_items"
args = []
"#,
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_hw.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Failed,
		"@checker('get_items') should detect 99 not in DATA"
	);
	assert!(
		alice.test_results[0].cases[0]
			.failure
			.as_ref()
			.unwrap()
			.message
			.contains("99"),
		"Error message should mention the invalid item"
	);
}

#[tokio::test]
async fn test_chain_setup_failure() {
	let dir = tempfile::tempdir().unwrap();

	let teacher_path = write_file(dir.path(), "teacher.py", "GREETING = 'hello'\n");

	// Student has no load_data function
	write_file(
		dir.path(),
		"alice_hw.py",
		r#"
def process(data):
    return len(data)
"#,
	);

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "setup_fail"
file = "hw.py"
language = "python"
imports = ["{teacher_path}"]

[[setup]]
id = "data"
function = "load_data"
args = ["test.csv"]

[[cases]]
name = "should be skipped"
function = "process"
args = ["$data"]
expect = 10
"#,
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_hw.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Error,
		"All cases should error when setup fails"
	);
	let msg = &alice.test_results[0].cases[0]
		.failure
		.as_ref()
		.unwrap()
		.message;
	assert!(
		msg.contains("setup") || msg.contains("Setup") || msg.contains("not found"),
		"Error should mention setup failure, got: {msg}"
	);
}

#[tokio::test]
async fn test_chain_copy_refs_prevents_mutation() {
	let dir = tempfile::tempdir().unwrap();

	let teacher_path = write_file(dir.path(), "teacher.py", "DATA = [1, 2, 3, 4, 5]\n");

	// Student mutates the input!
	write_file(
		dir.path(),
		"alice_hw.py",
		r#"
def pop_and_sum(data):
    data.pop()  # mutates!
    return sum(data)

def length(data):
    return len(data)
"#,
	);

	let spec: TestSpec = toml::from_str(&format!(
		r#"
[meta]
name = "mutation_test"
file = "hw.py"
language = "python"
imports = ["{teacher_path}"]

[[cases]]
name = "pop_and_sum"
function = "pop_and_sum"
args = ["$DATA"]
expect = 10

[[cases]]
name = "length after mutation"
function = "length"
args = ["$DATA"]
expect = 5
"#,
	))
	.unwrap();

	let executor = PythonExecutor::new();
	let submissions = SubmissionSet {
		by_student: HashMap::from([(
			"alice".to_string(),
			vec![StudentFile {
				path: dir.path().join("alice_hw.py"),
				language: "python".to_string(),
			}],
		)]),
	};

	let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
	let alice = &results["alice"];

	// With copy_refs=true (default), second case should still see original DATA
	assert_eq!(
		alice.test_results[0].cases[0].status,
		TestStatus::Passed,
		"pop_and_sum should work on copied data"
	);
	assert_eq!(
		alice.test_results[0].cases[1].status,
		TestStatus::Passed,
		"length should still be 5 because DATA was deepcopied per case"
	);
}
