use serde::{Deserialize, Serialize};

use crate::models::{Aggregation, Curve, SubmissionOutcome};

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
	/// A file/function decision needs teacher review; never a student zero.
	Matching,
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
	/// A checker failed: on the student's answer (student), or could not run (environment),
	/// or was still running when the process stopped (teacher).
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
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub matching: Option<crate::matching::Decision>,
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
	/// `[meta] name`.
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
/// Results written before grades were scored per item are refused, not reinterpreted:
/// unknown fields fail to parse, and `submission_state` is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudentReport {
	#[serde(default)]
	pub matches: Vec<crate::matching::ItemMatch>,
	/// `StudentKey`'s rendering — a bare 学号, or a `canvas:` / `local:` prefixed form that
	/// can never be mistaken for one.
	pub student_id: String,
	#[serde(default)]
	pub student_name: Option<String>,
	#[serde(default)]
	pub test_results: Vec<TestResult>,
	#[serde(default)]
	pub backend_name: Option<String>,
	/// Canvas user id, kept separate from `student_id` so grade push never has to guess it
	/// by parsing the student id as an integer.
	#[serde(default)]
	pub canvas_user_id: Option<u64>,
	/// How the submission arrived.
	pub submission_state: SubmissionOutcome,
	/// The source excused this student. A teacher's decision, so never a number.
	#[serde(default)]
	pub excused: bool,
	/// The style check, when a spec declares `[lint]`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub lint: Option<LintOutcome>,
	/// An infrastructure failure that stopped this student being graded at all — a panicked
	/// task, not a wrong answer. Kept out of `test_results` so it can never be counted as a
	/// failed test case and scored.
	#[serde(default)]
	pub error: Option<String>,
	/// What was graded: the attempt and the content of its files, so that evidence is never
	/// reused for a submission that has since changed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub submission: Option<SubmissionVersion>,
	/// The grade and how it was reached. `None` until scored: `run` writes evidence only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub grade: Option<Grade>,
}

impl StudentReport {
	/// A report with no evidence yet.
	pub fn new(student_id: impl Into<String>, submission_state: SubmissionOutcome) -> Self {
		Self {
			student_id: student_id.into(),
			matches: Vec::new(),
			student_name: None,
			test_results: Vec::new(),
			backend_name: None,
			canvas_user_id: None,
			submission_state,
			excused: false,
			lint: None,
			error: None,
			submission: None,
			grade: None,
		}
	}

	/// The number to publish: `None` when unscored or withheld.
	pub fn final_grade(&self) -> Option<f64> {
		self.grade.as_ref().and_then(Grade::final_grade)
	}

	/// The lint score, when the tool ran on a file.
	pub fn lint_score(&self) -> Option<f64> {
		match self.lint {
			Some(LintOutcome::Scored { score }) => Some(score),
			_ => None,
		}
	}

	pub fn total_cases(&self) -> usize {
		self.test_results.iter().map(|t| t.total()).sum()
	}

	pub fn total_passed(&self) -> usize {
		self.test_results.iter().map(|t| t.passed()).sum()
	}

	pub fn total_failed(&self) -> usize {
		self.total_cases() - self.total_passed()
	}

	/// Share of all cases passed. Informational only: grades come from items, where a
	/// case's weight never depends on how many cases its item runs.
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

/// The selected attempt of a submission, as its bytes stood when it was graded. Identity,
/// delivery state and excusal are the report's own fields; this holds only what they lack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionVersion {
	/// `None` when nothing was received.
	pub attempt: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub submitted_at: Option<String>,
	/// Every runnable file of the attempt, by path.
	pub files: Vec<FileVersion>,
	/// The archives those files were extracted from: a replaced archive is a changed
	/// submission even when the files graded out of it are byte-identical.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub archives: Vec<FileVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileVersion {
	pub path: std::path::PathBuf,
	pub sha256: String,
}

/// What the style check found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LintOutcome {
	/// 0–100.
	Scored { score: f64 },
	/// The linted item's file was not handed in.
	NoFile,
	/// The tool did not run properly: the machine's or the bundle's problem, never a score.
	Failed { message: String },
}

/// Why a grade was withheld, or why a graded 0 is a policy 0 rather than wrong answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
	NotSubmitted,
	SubmittedEmpty,
	Excused,
	/// The submission could not be matched to a roster student.
	PendingReview,
	/// Grading this student failed outright.
	GradingTaskFailed,
	/// An item's file was not handed in.
	MissingFile,
	TeacherFault,
	EnvironmentFault,
	/// The declared formula failed, or gave a value outside `0..=scale`, for this student.
	FormulaError,
	/// The lint tool did not run properly.
	LintFailed,
}

/// A student's grade and its basis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grade {
	#[serde(flatten)]
	pub outcome: GradeOutcome,
	/// Every item's points, plus lint points when declared.
	pub max: f64,
	/// One per declared item, in declaration order.
	pub items: Vec<ItemScore>,
	pub basis: GradeBasis,
}

impl Grade {
	pub fn final_grade(&self) -> Option<f64> {
		match self.outcome {
			GradeOutcome::Graded { final_grade, .. } => Some(final_grade),
			GradeOutcome::Withheld { .. } => None,
		}
	}

	pub fn reason(&self) -> Option<Reason> {
		match self.outcome {
			GradeOutcome::Graded { reason, .. } => reason,
			GradeOutcome::Withheld { reason, .. } => Some(reason),
		}
	}

	pub fn is_withheld(&self) -> bool {
		matches!(self.outcome, GradeOutcome::Withheld { .. })
	}
}

/// A number, or why there is none. There is no partial total: a grade missing one item
/// would reach Canvas looking like a real low grade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradeOutcome {
	Graded {
		/// Sum of item scores, unrounded.
		score: f64,
		/// `score / max * scale`, rounded.
		raw_grade: f64,
		/// The curved grade, rounded; equal to `raw_grade` without a curve.
		final_grade: f64,
		/// Set when this is a policy zero, so it never reads as wrong answers.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		reason: Option<Reason>,
	},
	Withheld {
		reason: Reason,
		/// The formula's error, when that is the reason.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		detail: Option<String>,
	},
}

/// The policy a grade was reached under, so it can be explained without the config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeBasis {
	pub scale: f64,
	pub decimals: u8,
	pub curve: Curve,
	/// Items were derived from the specs at 1 point each, not declared.
	pub derived_items: bool,
}

/// One item's score for one student.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemScore {
	pub item_id: String,
	pub points: u32,
	pub aggregation: Aggregation,
	pub passed: usize,
	pub cases: usize,
	#[serde(flatten)]
	pub outcome: ItemOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ItemOutcome {
	Graded {
		/// Unrounded.
		score: f64,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		reason: Option<Reason>,
	},
	Withheld {
		reason: Reason,
		/// The case that decided it, and why it did not pass.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		blocking_case: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		blocking_cause: Option<Cause>,
	},
}

/// Reports with a grade already on them, for tests that are about what happens to a grade
/// rather than how it was reached.
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
	use super::*;

	fn basis() -> GradeBasis {
		GradeBasis {
			scale: 100.0,
			decimals: 2,
			curve: Curve::Raw,
			derived_items: true,
		}
	}

	pub fn graded(student_id: &str, final_grade: f64) -> StudentReport {
		StudentReport {
			grade: Some(Grade {
				outcome: GradeOutcome::Graded {
					score: final_grade / 100.0,
					raw_grade: final_grade,
					final_grade,
					reason: None,
				},
				max: 1.0,
				items: Vec::new(),
				basis: basis(),
			}),
			..StudentReport::new(student_id, SubmissionOutcome::Executable)
		}
	}

	pub fn withheld(student_id: &str, state: SubmissionOutcome, reason: Reason) -> StudentReport {
		StudentReport {
			grade: Some(Grade {
				outcome: GradeOutcome::Withheld {
					reason,
					detail: None,
				},
				max: 1.0,
				items: Vec::new(),
				basis: basis(),
			}),
			..StudentReport::new(student_id, state)
		}
	}
}
