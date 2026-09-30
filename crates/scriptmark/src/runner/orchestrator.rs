use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::models::{
	CaseResult, Cause, FailureDetail, Fault, LintOutcome, StudentReport, StudentSubmission,
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
		let excused = student.is_excused();
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
			let mut report = StudentReport::new(sid, outcome);
			if runnable {
				run_student(&mut report, files, bundles, executor, semaphore, python).await;
			}
			// Nothing to run, but the student still gets a row.
			report.student_name = identity.name.clone();
			report.canvas_user_id = identity.canvas_user_id;
			report.excused = excused;
			report
		});
		handles.push((student, handle));
	}

	let mut reports = Vec::with_capacity(handles.len());
	for (student, handle) in handles {
		match handle.await {
			Ok(report) => reports.push(report),
			// A panicked task must not make the student disappear — but it must not look
			// like a failed test case either. Recorded as an error, so grading withholds a
			// grade rather than scoring an infrastructure failure.
			Err(e) => reports.push(StudentReport {
				student_name: student.identity.name.clone(),
				canvas_user_id: student.identity.canvas_user_id,
				excused: student.is_excused(),
				error: Some(format!("grading task failed: {e}")),
				..StudentReport::new(student.identity.key.to_string(), student.outcome())
			}),
		}
	}
	reports
}

/// Run every unit of every bundle for one student, and lint what a spec asks to.
async fn run_student<E: Executor>(
	report: &mut StudentReport,
	files: Vec<crate::models::StudentFile>,
	bundles: Arc<[Bundle]>,
	executor: Arc<E>,
	semaphore: Arc<Semaphore>,
	python: Arc<str>,
) {
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

	report.lint = lint(&bundles, &graded_files, &semaphore).await;
	report.test_results = bundles
		.iter()
		.zip(slots)
		.zip(graded_files)
		.map(|((bundle, units), file)| TestResult {
			item_id: bundle.spec.meta.name.clone(),
			file,
			cases: units.into_iter().flatten().flatten().collect(),
		})
		.collect();
	report.backend_name = Some(executor.language().to_string());
}

/// Lint the file of the first spec that asks for it — that item's file, not whichever the
/// student happened to hand in first.
async fn lint(
	bundles: &[Bundle],
	graded_files: &[Option<String>],
	semaphore: &Semaphore,
) -> Option<LintOutcome> {
	let (b, config) = bundles
		.iter()
		.enumerate()
		.find_map(|(b, bundle)| bundle.spec.lint.clone().map(|c| (b, c)))?;
	let Some(file) = graded_files[b].clone() else {
		return Some(LintOutcome::NoFile);
	};
	let _permit = semaphore.acquire().await.expect("semaphore closed");
	let outcome = tokio::task::spawn_blocking(move || {
		crate::runner::linter::run_lint(&config, std::path::Path::new(&file))
	})
	.await
	.map_err(|e| format!("linting failed: {e}"))
	.and_then(|scored| scored);
	Some(match outcome {
		Ok(score) => LintOutcome::Scored { score },
		Err(message) => LintOutcome::Failed { message },
	})
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
