//! The example bundles grade exactly as a teacher would run them: specs from `tests/`,
//! submissions discovered from `submissions/`, prepared, then run. They are the
//! reference for the three fixture kinds P-674 names — a pure function, a shared object,
//! and files read and written.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::models::{Cause, Fault, StudentReport, TestStatus};
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::prepare;
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::{load_spec, load_specs_from_dir};

fn examples() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

async fn grade_example(name: &str) -> Vec<StudentReport> {
	let root = examples().join("bundles").join(name);
	let specs = load_specs_from_dir(&root.join("tests")).unwrap_or_else(|e| panic!("{e}"));
	let submissions = root.join("submissions");
	let input = load_local_input(&[submissions.as_path()], LocalInputOptions::default()).unwrap();
	let executor = Arc::new(PythonExecutor::new());
	let bundles = prepare(specs, executor.clone(), 5)
		.await
		.unwrap_or_else(|e| panic!("{e}"));
	run_all(
		&input.students,
		bundles.into(),
		executor,
		&RunOptions::default(),
	)
	.await
}

fn student<'a>(reports: &'a [StudentReport], key: &str) -> &'a StudentReport {
	reports
		.iter()
		.find(|r| r.student_id.ends_with(key))
		.unwrap_or_else(|| panic!("no report for '{key}'"))
}

/// `(case name, status, cause)` for every case that did not pass.
fn failures(report: &StudentReport) -> Vec<(String, TestStatus, Option<Cause>)> {
	report
		.test_results
		.iter()
		.flat_map(|t| &t.cases)
		.filter(|c| c.status != TestStatus::Passed)
		.map(|c| (c.case_name.clone(), c.status, c.cause))
		.collect()
}

fn assert_all_passed(report: &StudentReport) {
	assert!(
		failures(report).is_empty(),
		"{} should pass everything: {:#?}",
		report.student_id,
		report.test_results
	);
}

#[tokio::test]
async fn test_the_pure_function_example() {
	let reports = grade_example("pure_function").await;
	assert_all_passed(student(&reports, "alice"));
	assert_eq!(
		failures(student(&reports, "bob")),
		[
			(
				"mean is not rounded".into(),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
			(
				"mean of nothing".into(),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
		]
	);
}

#[tokio::test]
async fn test_the_shared_object_example() {
	let reports = grade_example("shared_object").await;
	assert_all_passed(student(&reports, "alice"));
	let bob = student(&reports, "bob");
	let step = |name: &str| format!("ada's account / {name}");
	assert_eq!(
		failures(bob),
		[
			(
				step("deposit returns the new balance"),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
			(
				step("overdrawing is refused"),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
			(
				step("a refused withdrawal leaves the balance alone"),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
			(
				step("history records every change"),
				TestStatus::Failed,
				Some(Cause::Rejected)
			),
		],
		"the teacher's checker rejected a malformed answer; every failure is bob's"
	);
	assert!(
		bob.test_results[0]
			.cases
			.iter()
			.all(|c| c.fault != Some(Fault::Teacher)),
		"a wrong answer is never the teacher's fault"
	);
}

#[tokio::test]
async fn test_the_file_io_example() {
	let reports = grade_example("file_io").await;
	assert_all_passed(student(&reports, "alice"));
	assert_eq!(
		failures(student(&reports, "bob")),
		[(
			"report / write the report".into(),
			TestStatus::Failed,
			Some(Cause::Wrong)
		)]
	);
}

#[test]
fn test_the_standalone_example_spec_still_loads() {
	load_spec(&examples().join("python/test_larger_number.toml")).unwrap_or_else(|e| panic!("{e}"));
}
