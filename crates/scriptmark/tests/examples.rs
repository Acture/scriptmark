//! The example bundles grade exactly as a teacher would run them: specs from `tests/`,
//! submissions discovered from `submissions/`, prepared, then run. They are the
//! reference for the three fixture kinds P-674 names — a pure function, a shared object,
//! and files read and written — for generated cases (P-675), and for scoring them:
//! declared points, all or nothing under a curve, and items derived when nothing is
//! declared.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::models::{Cause, Fault, GradeOutcome, StudentReport, TestStatus};
use scriptmark::runner::frozen::Generation;
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::prepare;
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::{load_spec, load_specs_from_dir};

fn examples() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

async fn grade_example(name: &str) -> Vec<StudentReport> {
	grade_bundle(&examples().join("bundles").join(name)).await
}

#[tokio::test]
async fn test_reference_oracle_example() {
	let reports: Vec<StudentReport> = grade_example("reference_oracle").await;
	assert_all_passed(student(&reports, "alice"));
	assert_eq!(student(&reports, "alice").total_passed(), 8);
	assert_eq!(student(&reports, "bob").total_passed(), 0);
}

async fn grade_bundle(root: &Path) -> Vec<StudentReport> {
	let specs = load_specs_from_dir(&root.join("tests")).unwrap_or_else(|e| panic!("{e}"));
	let mut declared = scriptmark::assignment::load(None, &root.join("tests")).unwrap();
	let policy =
		scriptmark::assignment::settle(&mut declared.assignment, &declared.grading, &specs)
			.unwrap_or_else(|e| panic!("{e}"));
	let submissions = root.join("submissions");
	let input = load_local_input(&[submissions.as_path()], LocalInputOptions::default()).unwrap();
	let executor = Arc::new(PythonExecutor::new());
	let bundles = prepare(specs, &Generation::fresh(), executor.clone(), 5)
		.await
		.unwrap_or_else(|e| panic!("{e}"));
	let mut reports = run_all(
		&input.students,
		bundles.into(),
		executor,
		&RunOptions::default(),
	)
	.await;
	scriptmark::grading::grade_all(&mut reports, &declared.assignment.items, &policy)
		.unwrap_or_else(|e| panic!("{e}"));
	reports
}

/// `(score, raw_grade, final_grade)` of a graded student.
fn graded(report: &StudentReport) -> (f64, f64, f64) {
	match report.grade.as_ref().map(|g| &g.outcome) {
		Some(GradeOutcome::Graded {
			score,
			raw_grade,
			final_grade,
			..
		}) => (*score, *raw_grade, *final_grade),
		other => panic!("{} is not graded: {other:?}", report.student_id),
	}
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
	// 10 points on the item's pass rate.
	assert_eq!(graded(student(&reports, "alice")), (10.0, 100.0, 100.0));
	assert_eq!(graded(student(&reports, "bob")), (6.0, 60.0, 60.0));
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
	// All or nothing, curved onto 60..=100 — with the raw grade kept beside it.
	assert_eq!(graded(student(&reports, "alice")), (5.0, 100.0, 100.0));
	assert_eq!(graded(bob), (0.0, 0.0, 60.0));
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
	// No assignment.toml: the one spec is an item worth 1 point.
	assert_eq!(graded(student(&reports, "alice")), (1.0, 100.0, 100.0));
	let (score, raw, final_grade) = graded(student(&reports, "bob"));
	assert!((score - 2.0 / 3.0).abs() < 1e-9);
	assert_eq!((raw, final_grade), (66.67, 66.67));
}

#[tokio::test]
async fn test_the_generated_cases_example() {
	let reports = grade_example("generated_cases").await;
	assert_all_passed(student(&reports, "alice"));
	// Carol swaps the answers exactly on the bounds: whatever is drawn, only the two
	// samples that sit on them catch her.
	assert_eq!(
		failures(student(&reports, "carol")),
		[
			(
				"clamp [sample 0]".into(),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
			(
				"clamp [sample 1]".into(),
				TestStatus::Failed,
				Some(Cause::Wrong)
			),
		]
	);
	// 4 points on the pass rate: 14 of 16 cases is 3.5.
	assert_eq!(graded(student(&reports, "alice")), (4.0, 100.0, 100.0));
	assert_eq!(graded(student(&reports, "carol")), (3.5, 87.5, 87.5));
}

/// However many cases a template draws, the item is worth what it declares.
#[tokio::test]
async fn test_the_number_of_draws_never_changes_an_items_points() {
	let source = examples().join("bundles/generated_cases");
	for count in [12, 52] {
		let dir = tempfile::tempdir().unwrap();
		for sub in ["tests", "submissions"] {
			std::fs::create_dir_all(dir.path().join(sub)).unwrap();
			for entry in std::fs::read_dir(source.join(sub)).unwrap() {
				let entry = entry.unwrap();
				let text = std::fs::read_to_string(entry.path())
					.unwrap()
					.replace("count = 12", &format!("count = {count}"));
				std::fs::write(dir.path().join(sub).join(entry.file_name()), text).unwrap();
			}
		}
		std::fs::copy(
			source.join("assignment.toml"),
			dir.path().join("assignment.toml"),
		)
		.unwrap();
		let reports = grade_bundle(dir.path()).await;
		let cases = count + 4;
		let alice = student(&reports, "alice");
		assert_eq!(alice.total_cases(), cases);
		assert_eq!(alice.grade.as_ref().unwrap().max, 4.0, "count = {count}");
		assert_eq!(graded(alice), (4.0, 100.0, 100.0));
		let carol = student(&reports, "carol");
		assert_eq!(carol.grade.as_ref().unwrap().max, 4.0, "count = {count}");
		assert_eq!(
			graded(carol).0,
			4.0 * (cases - 2) as f64 / cases as f64,
			"count = {count}"
		);
	}
}

#[test]
fn test_the_standalone_example_spec_still_loads() {
	load_spec(&examples().join("python/test_larger_number.toml")).unwrap_or_else(|e| panic!("{e}"));
}
