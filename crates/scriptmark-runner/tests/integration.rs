use std::collections::HashMap;

use scriptmark_core::models::*;
use scriptmark_runner::orchestrator;
use scriptmark_runner::python::PythonExecutor;

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
