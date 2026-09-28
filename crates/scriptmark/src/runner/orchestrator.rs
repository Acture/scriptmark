use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::models::{
	CaseResult, Cause, FailureDetail, Fault, StudentFile, StudentReport, StudentSubmission,
	SubmissionState, TestResult, TestStatus,
};
use crate::runner::executor::Executor;
use crate::runner::judge::judge;
use crate::runner::prepare::{Bundle, Unit};

/// How a batch runs.
#[derive(Debug, Clone)]
pub struct RunOptions {
	/// Units running at once. Defaults to the number of CPUs; 0 runs one at a time, and a
	/// value past what a semaphore can count is capped there.
	pub concurrency: Option<usize>,
	/// The interpreter that runs teacher Python checker scripts.
	pub python: String,
}

impl Default for RunOptions {
	fn default() -> Self {
		Self {
			concurrency: None,
			python: "python3".into(),
		}
	}
}

/// Run every prepared bundle against every student.
///
/// Takes `&[StudentSubmission]` rather than the whole `AssignmentInput` so that the roster,
/// the unmatched artifacts and the diagnostics stay out of the runner.
///
/// Returns one report per student **in input order**, including students with nothing to
/// run: a roster member who did not submit must not vanish from the results, and a
/// `HashMap` keyed on student id would additionally drop one of two retained duplicates.
///
/// Each unit — an independent case or a scenario — is its own task holding one permit, so
/// a student's cases run concurrently; results come back in declaration order.
pub async fn run_all<E: Executor>(
	students: &[StudentSubmission],
	bundles: Arc<[Bundle]>,
	executor: Arc<E>,
	options: &RunOptions,
) -> Vec<StudentReport> {
	let concurrency = options
		.concurrency
		.unwrap_or_else(|| {
			std::thread::available_parallelism()
				.map(|n| n.get())
				.unwrap_or(4)
		})
		.clamp(1, Semaphore::MAX_PERMITS);
	let semaphore = Arc::new(Semaphore::new(concurrency));
	let python: Arc<str> = options.python.as_str().into();

	let mut handles = Vec::new();
	for student in students {
		let identity = student.identity.clone();
		let outcome = student.outcome();
		// Gate on the delivery axis, not the collapsed outcome: a submitter who is missing
		// from the roster still has runnable code, and refusing to run it would hide the
		// very output a teacher needs to resolve the mismatch.
		let runnable = student.state == SubmissionState::Executable;
		let files = student.files().to_vec();
		let (bundles, executor, semaphore, python) = (
			bundles.clone(),
			executor.clone(),
			semaphore.clone(),
			python.clone(),
		);

		let handle = tokio::spawn(async move {
			let sid = identity.key.to_string();
			let mut report = if runnable {
				run_student(sid, files, bundles, executor, semaphore, python).await
			} else {
				// Nothing to run, but the student still gets a row.
				StudentReport {
					student_id: sid,
					..Default::default()
				}
			};
			report.student_name = identity.name.clone();
			report.canvas_user_id = identity.canvas_user_id;
			report.submission_state = Some(outcome);
			report
		});
		handles.push((student, handle));
	}

	let mut reports = Vec::with_capacity(handles.len());
	for (student, handle) in handles {
		match handle.await {
			Ok(report) => reports.push(report),
			// A panicked task must not make the student disappear — but it must not look
			// like a failed test case either. Recorded as an error, so `is_gradeable()`
			// withholds a grade rather than scoring an infrastructure failure.
			Err(e) => reports.push(StudentReport {
				student_id: student.identity.key.to_string(),
				student_name: student.identity.name.clone(),
				canvas_user_id: student.identity.canvas_user_id,
				submission_state: Some(student.outcome()),
				error: Some(format!("grading task failed: {e}")),
				..Default::default()
			}),
		}
	}
	reports
}

/// Run every unit of every bundle for one student.
async fn run_student<E: Executor>(
	sid: String,
	files: Vec<StudentFile>,
	bundles: Arc<[Bundle]>,
	executor: Arc<E>,
	semaphore: Arc<Semaphore>,
	python: Arc<str>,
) -> StudentReport {
	let mut slots: Vec<Vec<Option<Vec<CaseResult>>>> =
		bundles.iter().map(|b| vec![None; b.units.len()]).collect();
	let mut tasks = JoinSet::new();
	let mut where_is = HashMap::new();
	let mut graded_files: Vec<Option<String>> = vec![None; bundles.len()];

	for (b, bundle) in bundles.iter().enumerate() {
		// One file per (student, bundle): every unit of a spec runs against the same file.
		let Some(file) = executor.locate(&files, &bundle.spec) else {
			for (u, unit) in bundle.units.iter().enumerate() {
				slots[b][u] = Some(blanket(
					unit,
					TestStatus::Missing,
					Fault::Student,
					Cause::NoFile,
					&format!(
						"No file matching '{}' found in submission",
						bundle.spec.meta.file
					),
				));
			}
			continue;
		};
		graded_files[b] = Some(file.path.to_string_lossy().into_owned());
		let path = std::path::absolute(&file.path).unwrap_or_else(|_| file.path.clone());
		for u in 0..bundle.units.len() {
			let (bundles, executor, semaphore, python, path) = (
				bundles.clone(),
				executor.clone(),
				semaphore.clone(),
				python.clone(),
				path.clone(),
			);
			let handle = tasks.spawn(async move {
				let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
				let mut plan = bundles[b].units[u].plan.clone();
				plan.file = path;
				let observation = executor.run(&plan).await;
				// Judging may run a teacher's checker script: keep it off the async workers.
				tokio::task::spawn_blocking(move || {
					let unit = &bundles[b].units[u];
					judge(&plan, &unit.scored(), &observation, &python)
				})
				.await
				.expect("judging panicked")
			});
			where_is.insert(handle.id(), (b, u));
		}
	}

	while let Some(joined) = tasks.join_next_with_id().await {
		match joined {
			Ok((id, results)) => {
				let (b, u) = where_is[&id];
				slots[b][u] = Some(results);
			}
			// A panicking unit costs that unit, not the student's other results.
			Err(e) => {
				let (b, u) = where_is[&e.id()];
				slots[b][u] = Some(blanket(
					&bundles[b].units[u],
					TestStatus::Error,
					Fault::Environment,
					Cause::Harness,
					&format!("grading this unit failed: {e}"),
				));
			}
		}
	}

	let test_results = bundles
		.iter()
		.zip(slots)
		.zip(graded_files)
		.map(|((bundle, units), file)| TestResult {
			item_id: bundle.spec.meta.name.clone(),
			file,
			cases: units.into_iter().flatten().flatten().collect(),
		})
		.collect();

	StudentReport {
		student_id: sid,
		test_results,
		backend_name: Some(executor.language().to_string()),
		lint_score: lint(&bundles, &files, &semaphore).await,
		..Default::default()
	}
}

/// Style score from the first spec that asks for one.
async fn lint(bundles: &[Bundle], files: &[StudentFile], semaphore: &Semaphore) -> Option<f64> {
	let config = bundles.iter().find_map(|b| b.spec.lint.clone())?;
	let file = files.first()?.path.clone();
	let _permit = semaphore.acquire().await.ok()?;
	tokio::task::spawn_blocking(move || crate::runner::linter::run_lint(&config, &file).style_score)
		.await
		.ok()
}

fn blanket(
	unit: &Unit,
	status: TestStatus,
	fault: Fault,
	cause: Cause,
	message: &str,
) -> Vec<CaseResult> {
	unit.scored
		.iter()
		.map(|(name, _)| CaseResult {
			case_name: name.clone(),
			status,
			failure: Some(FailureDetail {
				message: message.to_string(),
				details: String::new(),
			}),
			elapsed_ms: Some(0),
			fault: Some(fault),
			cause: Some(cause),
			..Default::default()
		})
		.collect()
}
