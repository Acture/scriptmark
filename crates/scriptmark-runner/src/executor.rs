use std::future::Future;

use scriptmark_core::models::{CaseResult, StudentFile, TestCase, TestSpec};

/// Language-specific execution logic.
///
/// Each language has one implementation that knows how to invoke student code
/// and collect results.
pub trait Executor: Send + Sync {
    /// Language identifier (e.g. "python", "cpp").
    fn language(&self) -> &str;

    /// Execute a single test case for a single student file.
    fn execute_case(
        &self,
        student_file: &StudentFile,
        spec: &TestSpec,
        case: &TestCase,
        timeout_secs: u64,
    ) -> impl Future<Output = CaseResult> + Send;
}
