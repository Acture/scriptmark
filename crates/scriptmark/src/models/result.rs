use serde::{Deserialize, Serialize};

use crate::models::SubmissionOutcome;

/// Status of a single test case or an overall student report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
	#[default]
	Passed,
	Failed,
	Missing,
	Error,
	Timeout,
}

/// Whose a non-pass is: who has to act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
	/// The student's code did it.
	Student,
	/// The teacher's bundle did it: a checker that could not decide, a teacher module or
	/// setup function that failed, a case with nothing to judge.
	Teacher,
	/// Neither: the machine or the grader.
	Environment,
}

/// Why a case did not pass, in one word — so a scorer never parses a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
	/// No file matched the spec.
	NoFile,
	/// The function, method or attribute does not exist.
	NoTarget,
	/// The answer was wrong: value, exception, stdout or file.
	Wrong,
	/// A teacher checker raised `AssertionError` on the answer.
	Rejected,
	/// An exception nobody expected.
	Raised,
	/// The student file does not compile.
	Syntax,
	/// The student module raised while loading.
	Load,
	/// The value returned cannot be represented.
	Unserialisable,
	/// The call exceeded its timeout.
	Timeout,
	/// The process died, or was killed at its deadline, during this call.
	Killed,
	/// Not run: an earlier call in the unit hung or killed the process.
	NotRun,
	/// Not run: a value it needs was never produced.
	Dependency,
	/// Not run: a setup call failed.
	Setup,
	/// The record stream could not be trusted.
	Protocol,
	/// A teacher module failed to import.
	TeacherImport,
	/// A checker could not decide.
	Checker,
	/// The case declared nothing that was judged.
	NothingToJudge,
	/// The process could not be started.
	Spawn,
	/// The harness itself failed.
	Harness,
}

/// What the call was given: the evidence behind "明确测试输入".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaseInput {
	/// The name asked for, and — when a lookup may have substituted — the one it found.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resolved: Option<String>,
	/// Arguments as declared, with `$ref`s by name.
	#[serde(default)]
	pub args: Vec<serde_json::Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub stdin: Option<String>,
}

/// Detail about why a test case failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureDetail {
	pub message: String,
	#[serde(default)]
	pub details: String,
}

/// Result of a single test case for a single student.
///
/// `fault`, `cause`, `stdout` and `input` are absent from results written before they
/// existed, and must not be read as "no fault".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaseResult {
	pub case_name: String,
	pub status: TestStatus,
	/// What the student's code actually produced.
	pub actual: Option<String>,
	/// What was expected.
	pub expected: Option<String>,
	/// Failure detail if status != Passed.
	pub failure: Option<FailureDetail>,
	/// Execution time in milliseconds.
	pub elapsed_ms: Option<u64>,
	/// Whose the non-pass is. `None` when passed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fault: Option<Fault>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cause: Option<Cause>,
	/// What the student printed during the call, when anything.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub stdout: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub input: Option<CaseInput>,
}

/// Aggregated result for one grading item for one student.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
	/// The [`crate::models::GradingItem`] this evidence belongs to — the test spec's
	/// `[meta] name`. Read from `spec_name` in results written before items were modelled.
	#[serde(alias = "spec_name")]
	pub item_id: String,
	/// The student file these cases ran against, as submitted. `None` when no file matched,
	/// or in results written before it was recorded.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub file: Option<String>,
	pub cases: Vec<CaseResult>,
}

impl TestResult {
	pub fn total(&self) -> usize {
		self.cases.len()
	}

	pub fn passed(&self) -> usize {
		self.cases
			.iter()
			.filter(|c| c.status == TestStatus::Passed)
			.count()
	}

	pub fn failed(&self) -> usize {
		self.total() - self.passed()
	}

	pub fn pass_rate(&self) -> f64 {
		if self.total() == 0 {
			return 0.0;
		}
		(self.passed() as f64 / self.total() as f64) * 100.0
	}

	pub fn status(&self) -> TestStatus {
		if self.cases.is_empty() {
			return TestStatus::Missing;
		}
		if self.cases.iter().all(|c| c.status == TestStatus::Passed) {
			TestStatus::Passed
		} else {
			TestStatus::Failed
		}
	}
}

/// Complete report for a single student across all test specs.
///
/// New fields are `Option` rather than defaulted enums: a results file written before this
/// model existed must not claim a `submission_state` it never recorded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StudentReport {
	/// `StudentKey`'s rendering — a bare 学号, or a `canvas:` / `local:` prefixed form that
	/// can never be mistaken for one.
	pub student_id: String,
	#[serde(default)]
	pub student_name: Option<String>,
	#[serde(default)]
	pub test_results: Vec<TestResult>,
	#[serde(default)]
	pub final_grade: Option<f64>,
	#[serde(default)]
	pub backend_name: Option<String>,
	/// Lint-based style score (0-100). Set by linter, used by grading.
	#[serde(default)]
	pub lint_score: Option<f64>,
	/// Canvas user id, kept separate from `student_id` so grade push never has to guess it
	/// by parsing the student id as an integer.
	#[serde(default)]
	pub canvas_user_id: Option<u64>,
	/// How the submission arrived. `None` on records written before this field existed.
	#[serde(default)]
	pub submission_state: Option<SubmissionOutcome>,
	/// An infrastructure failure that stopped this student being graded at all — a panicked
	/// task, not a wrong answer. Kept out of `test_results` so it can never be counted as a
	/// failed test case and scored.
	#[serde(default)]
	pub error: Option<String>,
}

impl StudentReport {
	/// True when the student actually had runnable code and nothing went wrong running it —
	/// the only case a numeric grade means anything. Reports from before these fields
	/// existed are graded as they were.
	pub fn is_gradeable(&self) -> bool {
		self.error.is_none()
			&& matches!(
				self.submission_state,
				None | Some(SubmissionOutcome::Executable)
			)
	}
}

impl StudentReport {
	pub fn total_cases(&self) -> usize {
		self.test_results.iter().map(|t| t.total()).sum()
	}

	pub fn total_passed(&self) -> usize {
		self.test_results.iter().map(|t| t.passed()).sum()
	}

	pub fn total_failed(&self) -> usize {
		self.total_cases() - self.total_passed()
	}

	pub fn pass_rate(&self) -> f64 {
		let total = self.total_cases();
		if total == 0 {
			return 0.0;
		}
		(self.total_passed() as f64 / total as f64) * 100.0
	}

	pub fn status(&self) -> TestStatus {
		if self.test_results.is_empty() {
			return TestStatus::Missing;
		}
		if self
			.test_results
			.iter()
			.all(|t| t.status() == TestStatus::Passed)
		{
			TestStatus::Passed
		} else {
			TestStatus::Failed
		}
	}
}
