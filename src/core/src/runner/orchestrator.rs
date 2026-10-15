use std::collections::{BTreeMap, HashMap};
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
	pub matching: crate::matching::Config,
	/// Units running at once. Defaults to the number of CPUs; 0 runs one at a time, and a
	/// value past what a semaphore can count is capped there.
	pub concurrency: Option<usize>,
	/// The interpreter that runs teacher Python checker scripts.
	pub python: String,
	/// Whether teacher checker scripts and lint tools run in the teacher's sandbox, as the
	/// units do.
	pub isolation: crate::runner::sandbox::Isolation,
}

impl Default for RunOptions {
	fn default() -> Self {
		Self {
			matching: crate::matching::Config::default(),
			concurrency: None,
			python: "python3".into(),
			isolation: crate::runner::sandbox::Isolation::default(),
		}
	}
}

/// How teacher scripts and tools run outside a unit: checker scripts and lint.
#[derive(Debug, Clone)]
struct Tools {
	python: Arc<str>,
	isolation: crate::runner::sandbox::Isolation,
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
	let tools = Tools {
		python: options.python.as_str().into(),
		isolation: options.isolation,
	};
	let matching = Arc::new(options.matching.clone());

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
		let matches: Vec<crate::matching::ItemMatch> = bundles
			.iter()
			.map(|bundle| crate::matching::item_match(&matching, student, &bundle.spec))
			.collect();
		let matching = matching.clone();
		let (bundles, executor, semaphore, tools) = (
			bundles.clone(),
			executor.clone(),
			semaphore.clone(),
			tools.clone(),
		);

		let handle = tokio::spawn(async move {
			let sid = identity.key.to_string();
			let mut report = StudentReport::new(sid, outcome);
			report.matches = matches;
			if runnable {
				run_student(
					&mut report,
					files,
					bundles,
					executor,
					semaphore,
					tools,
					matching,
				)
				.await;
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
	tools: Tools,
	matching: Arc<crate::matching::Config>,
) {
	let mut slots: Vec<Vec<Option<Vec<CaseResult>>>> =
		bundles.iter().map(|b| vec![None; b.units.len()]).collect();
	let mut tasks = JoinSet::new();
	let mut where_is = HashMap::new();
	let mut graded_files: Vec<Option<String>> = vec![None; bundles.len()];
	let mut lookups: Vec<Vec<BTreeMap<String, crate::matching::Decision>>> = bundles
		.iter()
		.map(|bundle| vec![BTreeMap::new(); bundle.units.len()])
		.collect();

	for (b, bundle) in bundles.iter().enumerate() {
		// One file per (student, bundle): every unit of a spec runs against the same file.
		let decision = &report.matches[b].file;
		let Some(file) = files.iter().find(|file| {
			Some(file.path.to_string_lossy().as_ref()) == decision.selected.as_deref()
		}) else {
			let pending = decision.state != crate::matching::State::Missing
				|| matching.overrides.iter().any(|entry| {
					entry.student == report.student_id
						&& entry.item == bundle.spec.meta.name
						&& entry.file.is_some()
				});
			for (u, unit) in bundle.units.iter().enumerate() {
				slots[b][u] = Some(blanket(
					unit,
					if pending {
						TestStatus::Error
					} else {
						TestStatus::Missing
					},
					if pending {
						Fault::Teacher
					} else {
						Fault::Student
					},
					if pending {
						Cause::Matching
					} else {
						Cause::NoFile
					},
					&format!(
						"File matching '{}' requires {:?}: {:?}",
						bundle.spec.meta.file, decision.state, decision.candidates
					),
				));
			}
			continue;
		};
		graded_files[b] = Some(file.path.to_string_lossy().into_owned());
		let path = std::path::absolute(&file.path).unwrap_or_else(|_| file.path.clone());
		let functions = matching.functions(&report.student_id, &bundle.spec.meta.name);
		for u in 0..bundle.units.len() {
			let functions = functions.clone();
			let (bundles, executor, semaphore, tools, path) = (
				bundles.clone(),
				executor.clone(),
				semaphore.clone(),
				tools.clone(),
				path.clone(),
			);
			let handle = tasks.spawn(async move {
				let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
				let mut plan = bundles[b].units[u].plan.clone();
				plan.file = path;
				plan.functions = functions;
				let observation = executor.run(&plan).await;
				// Judging may run a teacher's checker script: keep it off the async workers.
				tokio::task::spawn_blocking(move || {
					let unit = &bundles[b].units[u];
					let mut functions: BTreeMap<String, crate::matching::Decision> =
						BTreeMap::new();
					for target in observation
						.setup
						.iter()
						.chain(&observation.steps)
						.filter_map(|record| record.target.as_ref())
					{
						if let Some(decision) = &target.matching {
							functions
								.entry(target.requested.clone())
								.and_modify(|current| current.merge(decision.clone()))
								.or_insert_with(|| decision.clone());
						}
					}
					(
						judge(
							&plan,
							&unit.scored(),
							&observation,
							&tools.python,
							tools.isolation,
						),
						functions,
					)
				})
				.await
				.expect("judging panicked")
			});
			where_is.insert(handle.id(), (b, u));
		}
	}

	while let Some(joined) = tasks.join_next_with_id().await {
		match joined {
			Ok((id, (results, functions))) => {
				let (b, u) = where_is[&id];
				slots[b][u] = Some(results);
				lookups[b][u] = functions;
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

	for (b, units) in lookups.into_iter().enumerate() {
		for (name, decision) in units.into_iter().flat_map(BTreeMap::into_iter) {
			report.matches[b]
				.functions
				.entry(name)
				.and_modify(|current| current.merge(decision.clone()))
				.or_insert(decision);
		}
		if report.matches[b].functions.values().any(|decision| {
			matches!(
				decision.state,
				crate::matching::State::Review | crate::matching::State::Ambiguous
			)
		}) {
			for cases in slots[b].iter_mut().flatten() {
				for case in cases {
					case.status = TestStatus::Error;
					case.fault = Some(Fault::Teacher);
					case.cause = Some(Cause::Matching);
					case.failure = Some(FailureDetail {
						message: format!(
							"function matching requires review: {:?}",
							report.matches[b].functions
						),
						details: String::new(),
					});
				}
			}
		}
	}
	report.lint = lint(&bundles, &graded_files, &semaphore, tools.isolation).await;
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

/// Lint every file a spec asks to — each item's own file, with its own `[lint]` — and score
/// the student by their mean: an item whose file is missing counts 0. A file two items
/// share under the same command is linted once.
async fn lint(
	bundles: &[Bundle],
	graded_files: &[Option<String>],
	semaphore: &Semaphore,
	isolation: crate::runner::sandbox::Isolation,
) -> Option<LintOutcome> {
	let mut asked: Vec<(
		(Option<String>, String),
		crate::models::LintConfig,
		std::path::PathBuf,
	)> = Vec::new();
	for (bundle, file) in bundles.iter().zip(graded_files) {
		if let Some(config) = &bundle.spec.lint {
			let key = (file.clone(), config.command.clone());
			if !asked.iter().any(|(k, ..)| *k == key) {
				asked.push((key, config.clone(), bundle.spec.dir.clone()));
			}
		}
	}
	if asked.is_empty() {
		return None;
	}
	if asked.iter().all(|((file, _), ..)| file.is_none()) {
		return Some(LintOutcome::NoFile);
	}
	let mut scores = Vec::new();
	let mut failures = Vec::new();
	for ((file, _), config, bundle) in asked {
		let Some(file) = file else {
			scores.push(0.0);
			continue;
		};
		let _permit = semaphore.acquire().await.expect("semaphore closed");
		let outcome = tokio::task::spawn_blocking(move || {
			crate::runner::linter::run_lint(
				&config,
				std::path::Path::new(&file),
				&bundle,
				isolation,
				crate::runner::linter::LINT_TIMEOUT,
			)
		})
		.await
		.map_err(|e| format!("linting failed: {e}"))
		.and_then(|scored| scored);
		match outcome {
			Ok(score) => scores.push(score),
			Err(message) => failures.push(message),
		}
	}
	Some(if failures.is_empty() {
		LintOutcome::Scored {
			score: scores.iter().sum::<f64>() / scores.len() as f64,
		}
	} else {
		LintOutcome::Failed {
			message: failures.join("; "),
		}
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
