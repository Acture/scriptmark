use serde::{Deserialize, Serialize};

/// Status of a single test case or an overall student report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
	Passed,
	Failed,
	Missing,
	Error,
	Timeout,
}

/// Detail about why a test case failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureDetail {
	pub message: String,
	#[serde(default)]
	pub details: String,
}

/// Result of a single test case for a single student.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Aggregated result for one test spec (one TOML file) for one student.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
	pub spec_name: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudentReport {
	pub student_id: String,
	#[serde(default)]
	pub student_name: Option<String>,
	pub test_results: Vec<TestResult>,
	#[serde(default)]
	pub final_grade: Option<f64>,
	#[serde(default)]
	pub backend_name: Option<String>,
	/// Lint-based style score (0-100). Set by linter, used by grading.
	#[serde(default)]
	pub lint_score: Option<f64>,
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
