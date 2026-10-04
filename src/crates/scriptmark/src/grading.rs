//! Turning evidence into grades, one item at a time.
//!
//! An item is worth its declared points however many cases it runs. A grade is a number
//! only when every item could be scored: a teacher or environment fault, or a policy that
//! says so, withholds it with a reason instead. Zeros come only from the student's own
//! evidence, a declared policy, or a declared curve.

use anyhow::{Result, bail};
use rhai::{AST, Dynamic, Engine, Scope};

use crate::checker::rhai_checker::engine;
use crate::models::{
	Aggregation, Cause, Curve, CurveTemplate, Fault, Grade, GradeBasis, GradeOutcome,
	GradingConfig, GradingItem, ItemOutcome, ItemScore, LintOutcome, MissingPolicy, Reason,
	StudentReport, SubmissionOutcome, TestResult, TestStatus,
};

/// The names a grading formula sees.
pub const FORMULA_VARIABLES: [&str; 4] = ["score", "max", "fraction", "scale"];

/// A grading policy that was checked, and whose formula was compiled, before any student
/// ran — so a broken policy refuses the batch instead of failing every student.
pub struct Policy {
	config: GradingConfig,
	derived_items: bool,
	formula: Option<(Engine, AST)>,
}

/// Everything wrong with a policy, found at once.
#[derive(Debug, thiserror::Error)]
#[error("refusing to grade: {}", .0.join("; "))]
pub struct PolicyError(pub Vec<String>);

impl Policy {
	/// Check a policy and compile its formula. `derived_items` records that the items were
	/// derived from the specs rather than declared.
	pub fn compile(config: GradingConfig, derived_items: bool) -> Result<Self, PolicyError> {
		let mut problems = Vec::new();
		if !config.scale.is_finite() || config.scale <= 0.0 {
			problems.push(format!(
				"[grading] scale must be a positive number, not {}",
				config.scale
			));
		}
		if config.decimals > 4 {
			problems.push(format!(
				"[grading] decimals must be 0 to 4, not {}",
				config.decimals
			));
		}
		let mut formula = None;
		match &config.curve {
			Curve::Raw => {}
			Curve::Template { lower, upper, .. } => {
				if !(lower.is_finite()
					&& upper.is_finite()
					&& 0.0 <= *lower
					&& lower <= upper
					&& *upper <= config.scale)
				{
					problems.push(format!(
						"[grading] curve needs 0 <= lower <= upper <= scale, got lower = {lower}, \
						 upper = {upper}, scale = {}",
						config.scale
					));
				}
			}
			Curve::Formula { formula: source } => {
				let mut engine = engine();
				engine.set_strict_variables(true);
				let mut scope = Scope::new();
				for name in FORMULA_VARIABLES {
					scope.push(name, 0.0_f64);
				}
				match engine.compile_with_scope(&scope, source) {
					Ok(ast) => formula = Some((engine, ast)),
					Err(e) => problems.push(format!(
						"[grading] formula `{source}` does not compile: {e}"
					)),
				}
			}
		}
		if !problems.is_empty() {
			return Err(PolicyError(problems));
		}
		Ok(Self {
			config,
			derived_items,
			formula,
		})
	}

	pub fn config(&self) -> &GradingConfig {
		&self.config
	}

	/// The items were derived from the specs, not declared.
	pub fn derived_items(&self) -> bool {
		self.derived_items
	}

	fn basis(&self) -> GradeBasis {
		GradeBasis {
			scale: self.config.scale,
			decimals: self.config.decimals,
			curve: self.config.curve.clone(),
			derived_items: self.derived_items,
		}
	}

	fn round(&self, x: f64) -> f64 {
		round_half_away(x, self.config.decimals)
	}

	/// The curved grade before rounding, or why there is none.
	fn curve(&self, score: f64, max: f64) -> Result<f64, String> {
		let scale = self.config.scale;
		let fraction = score / max;
		let value = match &self.config.curve {
			Curve::Raw => fraction * scale,
			Curve::Template { name, lower, upper } => {
				let span = upper - lower;
				match name {
					CurveTemplate::Linear => lower + fraction * span,
					CurveTemplate::Sqrt => lower + fraction.sqrt() * span,
					CurveTemplate::Log => {
						lower + (1.0 + 100.0 * fraction).ln() / 101f64.ln() * span
					}
					CurveTemplate::Strict if fraction >= 1.0 => *upper,
					CurveTemplate::Strict if fraction >= 0.8 => {
						lower + (fraction - 0.8) / 0.2 * span
					}
					CurveTemplate::Strict => *lower,
				}
			}
			Curve::Formula { .. } => {
				let (engine, ast) = self.formula.as_ref().expect("compiled with the policy");
				let mut scope = Scope::new();
				scope.push("score", score);
				scope.push("max", max);
				scope.push("fraction", fraction);
				scope.push("scale", scale);
				let value = engine
					.eval_ast_with_scope::<Dynamic>(&mut scope, ast)
					.map_err(|e| format!("the formula failed: {e}"))?;
				if let Ok(f) = value.as_float() {
					f
				} else if let Ok(i) = value.as_int() {
					i as f64
				} else {
					return Err(format!(
						"the formula returned {}, not a number",
						value.type_name()
					));
				}
			}
		};
		if !value.is_finite() || !(0.0..=scale).contains(&value) {
			return Err(format!(
				"the grade came out as {value}, outside 0 to {scale}"
			));
		}
		Ok(value)
	}
}

/// Round half away from zero to `decimals` places.
pub fn round_half_away(x: f64, decimals: u8) -> f64 {
	let factor = 10f64.powi(i32::from(decimals));
	(x * factor).round() / factor
}

/// Why a student's evidence is never scored: who they are, and whether anything arrived,
/// decide before any evidence does.
pub fn gate(report: &StudentReport) -> Option<Reason> {
	if report.excused {
		Some(Reason::Excused)
	} else if report.error.is_some() {
		Some(Reason::GradingTaskFailed)
	} else {
		match report.submission_state {
			SubmissionOutcome::ReceivedUnmatched => Some(Reason::PendingReview),
			SubmissionOutcome::NotSubmitted => Some(Reason::NotSubmitted),
			SubmissionOutcome::SubmittedEmpty => Some(Reason::SubmittedEmpty),
			SubmissionOutcome::Executable => None,
		}
	}
}

/// What lint earns of the `points` it is worth: `None` when the tool did not run properly,
/// which withholds the grade.
pub fn lint_earned(points: u32, lint: Option<&LintOutcome>) -> Option<f64> {
	match lint {
		Some(LintOutcome::Scored { score }) if score.is_finite() => {
			Some(f64::from(points) * score.clamp(0.0, 100.0) / 100.0)
		}
		// The linted item's file is missing, and `missing_file` already decided that item:
		// withheld, or a 0 — which is what lint earns too.
		Some(LintOutcome::NoFile) => Some(0.0),
		Some(LintOutcome::Scored { .. } | LintOutcome::Failed { .. }) | None => None,
	}
}

/// Score every report against `items`, replacing any earlier grade.
///
/// Each report is scored on its own evidence alone. Errs only when the evidence breaks
/// the runner's contract — an item with no results, or a non-pass with no owner — which
/// is a bug to fix, not a grade to guess.
pub fn grade_all(
	reports: &mut [StudentReport],
	items: &[GradingItem],
	policy: &Policy,
) -> Result<()> {
	let max = f64::from(
		items.iter().map(|i| i.points).sum::<u32>() + policy.config.lint_points.unwrap_or(0),
	);
	if max == 0.0 {
		bail!("refusing to grade: the items are worth 0 points in total");
	}
	for report in reports.iter_mut() {
		report.grade = Some(grade_one(report, items, policy, max)?);
	}
	Ok(())
}

fn grade_one(
	report: &StudentReport,
	items: &[GradingItem],
	policy: &Policy,
	max: f64,
) -> Result<Grade> {
	let grade = |outcome, items| Grade {
		outcome,
		max,
		items,
		basis: policy.basis(),
	};
	let withheld = |reason| GradeOutcome::Withheld {
		reason,
		detail: None,
	};

	if let Some(reason) = gate(report) {
		let policy_zero = matches!(reason, Reason::NotSubmitted | Reason::SubmittedEmpty)
			&& policy.config.missing == MissingPolicy::Zero;
		if !policy_zero {
			return Ok(grade(withheld(reason), Vec::new()));
		}
		let scores = items
			.iter()
			.map(|item| {
				item_score(
					item,
					0,
					0,
					ItemOutcome::Graded {
						score: 0.0,
						reason: Some(reason),
					},
				)
			})
			.collect();
		return Ok(grade(
			GradeOutcome::Graded {
				score: 0.0,
				raw_grade: 0.0,
				final_grade: 0.0,
				reason: Some(reason),
			},
			scores,
		));
	}

	let mut scores = Vec::with_capacity(items.len());
	for item in items {
		let mut results = report.test_results.iter().filter(|r| r.item_id == item.id);
		let (Some(result), None) = (results.next(), results.next()) else {
			bail!(
				"{}: item '{}' needs exactly one set of results; the runner makes one per spec",
				report.student_id,
				item.id
			);
		};
		scores.push(score_item(item, result, policy.config.missing_file)?);
	}

	let mut score: f64 = scores
		.iter()
		.map(|s| match s.outcome {
			ItemOutcome::Graded { score, .. } => score,
			ItemOutcome::Withheld { .. } => 0.0,
		})
		.sum();
	let mut lint_withheld = None;
	if let Some(points) = policy.config.lint_points {
		match lint_earned(points, report.lint.as_ref()) {
			Some(earned) => score += earned,
			None => lint_withheld = Some(Reason::LintFailed),
		}
	}

	let first_withheld = scores.iter().find_map(|s| match s.outcome {
		ItemOutcome::Withheld { reason, .. } => Some(reason),
		ItemOutcome::Graded { .. } => None,
	});
	if let Some(reason) = first_withheld.or(lint_withheld) {
		return Ok(grade(withheld(reason), scores));
	}

	// Nothing handed in for any item, zeroed by `missing_file`: a policy zero like
	// `missing`'s, so no curve lifts it and it says why.
	let all_missing = scores.iter().all(|s| {
		matches!(
			s.outcome,
			ItemOutcome::Graded {
				reason: Some(Reason::MissingFile),
				..
			}
		)
	});
	if all_missing {
		return Ok(grade(
			GradeOutcome::Graded {
				score: 0.0,
				raw_grade: 0.0,
				final_grade: 0.0,
				reason: Some(Reason::MissingFile),
			},
			scores,
		));
	}

	let outcome = match policy.curve(score, max) {
		Ok(curved) => GradeOutcome::Graded {
			score,
			raw_grade: policy.round(score / max * policy.config.scale),
			final_grade: policy.round(curved),
			reason: None,
		},
		Err(detail) => GradeOutcome::Withheld {
			reason: Reason::FormulaError,
			detail: Some(detail),
		},
	};
	Ok(grade(outcome, scores))
}

fn item_score(item: &GradingItem, passed: usize, cases: usize, outcome: ItemOutcome) -> ItemScore {
	ItemScore {
		item_id: item.id.clone(),
		points: item.points,
		aggregation: item.aggregation,
		passed,
		cases,
		outcome,
	}
}

/// Score one item from its cases, trusting each case's `fault`.
fn score_item(
	item: &GradingItem,
	result: &TestResult,
	missing_file: MissingPolicy,
) -> Result<ItemScore> {
	let cases = result.cases.len();
	let passed = result.passed();
	if cases == 0 {
		bail!(
			"item '{}' has no cases; specs with none are refused at load",
			item.id
		);
	}
	let failing = || {
		result
			.cases
			.iter()
			.filter(|c| c.status != TestStatus::Passed)
	};
	if let Some(case) = failing().find(|c| c.fault.is_none()) {
		bail!(
			"item '{}': case '{}' did not pass and names no owner; every non-pass must",
			item.id,
			case.case_name
		);
	}
	let blocked = |fault, reason| {
		failing()
			.find(|c| c.fault == Some(fault))
			.map(|case| ItemOutcome::Withheld {
				reason,
				blocking_case: Some(case.case_name.clone()),
				blocking_cause: case.cause,
			})
	};
	let outcome = if let Some(outcome) = blocked(Fault::Teacher, Reason::TeacherFault)
		.or_else(|| blocked(Fault::Environment, Reason::EnvironmentFault))
	{
		outcome
	} else if result.cases.iter().all(|c| c.cause == Some(Cause::NoFile)) {
		match missing_file {
			MissingPolicy::Withheld => ItemOutcome::Withheld {
				reason: Reason::MissingFile,
				blocking_case: None,
				blocking_cause: Some(Cause::NoFile),
			},
			MissingPolicy::Zero => ItemOutcome::Graded {
				score: 0.0,
				reason: Some(Reason::MissingFile),
			},
		}
	} else {
		let points = f64::from(item.points);
		let score = match item.aggregation {
			Aggregation::Proportional => points * passed as f64 / cases as f64,
			Aggregation::AllOrNothing if passed == cases => points,
			Aggregation::AllOrNothing => 0.0,
		};
		ItemOutcome::Graded {
			score,
			reason: None,
		}
	};
	Ok(item_score(item, passed, cases, outcome))
}

/// Things a teacher should look at after grading, though no grade depends on them: an item
/// every student failed through its checker, or where no student handed in the file.
/// Either usually means the bundle is wrong, not the class.
pub fn diagnostics(reports: &[StudentReport], items: &[GradingItem]) -> Vec<String> {
	let ran: Vec<&StudentReport> = reports
		.iter()
		.filter(|r| r.submission_state == SubmissionOutcome::Executable && r.error.is_none())
		.collect();
	if ran.is_empty() {
		return Vec::new();
	}
	let every = |item: &GradingItem, test: &dyn Fn(&TestResult) -> bool| {
		ran.iter().all(|r| {
			r.test_results
				.iter()
				.any(|t| t.item_id == item.id && test(t))
		})
	};
	let mut warnings = Vec::new();
	for item in items {
		if every(item, &|t| {
			t.cases.iter().all(|c| c.cause == Some(Cause::NoFile))
		}) {
			warnings.push(format!(
				"no student handed in the file for item '{}': check its spec's [meta] file",
				item.id
			));
		} else if every(item, &|t| {
			t.cases
				.iter()
				.any(|c| c.cause == Some(Cause::Checker) && c.fault == Some(Fault::Student))
		}) {
			warnings.push(format!(
				"every student failed item '{}' through its checker: check the checker",
				item.id
			));
		}
	}
	warnings
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{CaseResult, TestResult};

	fn case(name: &str, fault: Option<Fault>, cause: Option<Cause>) -> CaseResult {
		CaseResult {
			case_name: name.into(),
			status: if fault.is_some() {
				TestStatus::Failed
			} else {
				TestStatus::Passed
			},
			fault,
			cause,
			..Default::default()
		}
	}

	/// `passed` of `total` cases, the rest the student's wrong answers.
	fn result(item: &str, passed: usize, total: usize) -> TestResult {
		TestResult {
			item_id: item.into(),
			file: None,
			cases: (0..total)
				.map(|i| {
					if i < passed {
						case(&format!("c{i}"), None, None)
					} else {
						case(&format!("c{i}"), Some(Fault::Student), Some(Cause::Wrong))
					}
				})
				.collect(),
		}
	}

	fn student(results: Vec<TestResult>) -> StudentReport {
		StudentReport {
			test_results: results,
			..StudentReport::new("s", SubmissionOutcome::Executable)
		}
	}

	fn item(id: &str, points: u32) -> GradingItem {
		GradingItem {
			points,
			..GradingItem::new(id)
		}
	}

	fn policy(config: GradingConfig) -> Policy {
		Policy::compile(config, false).unwrap()
	}

	fn curve(curve: Curve) -> Policy {
		policy(GradingConfig {
			curve,
			..GradingConfig::default()
		})
	}

	fn grade(report: StudentReport, items: &[GradingItem], policy: &Policy) -> Grade {
		let mut reports = [report];
		grade_all(&mut reports, items, policy).unwrap();
		reports[0].grade.clone().unwrap()
	}

	fn withheld_reason(grade: &Grade) -> Option<Reason> {
		grade.is_withheld().then(|| grade.reason()).flatten()
	}

	#[test]
	fn test_more_cases_do_not_change_an_items_max() {
		let items = [item("a", 2), item("b", 1)];
		let p = policy(GradingConfig::default());
		let few = grade(
			student(vec![result("a", 4, 5), result("b", 1, 1)]),
			&items,
			&p,
		);
		let many = grade(
			student(vec![result("a", 40, 50), result("b", 1, 1)]),
			&items,
			&p,
		);
		assert_eq!(few.max, 3.0);
		assert_eq!(many.max, 3.0);
		assert_eq!(few.outcome, many.outcome);
		// 2 × 4/5 + 1 = 2.6 of 3.
		assert_eq!(few.final_grade(), Some(86.67));
	}

	#[test]
	fn test_all_correct_and_partial() {
		let items = [item("a", 1), item("b", 1)];
		let p = policy(GradingConfig::default());
		let all = grade(
			student(vec![result("a", 4, 4), result("b", 1, 1)]),
			&items,
			&p,
		);
		assert_eq!(all.final_grade(), Some(100.0));
		let partial = grade(
			student(vec![result("a", 3, 4), result("b", 1, 1)]),
			&items,
			&p,
		);
		assert_eq!(
			partial.outcome,
			GradeOutcome::Graded {
				score: 1.75,
				raw_grade: 87.5,
				final_grade: 87.5,
				reason: None,
			}
		);
	}

	#[test]
	fn test_all_or_nothing() {
		let items = [GradingItem {
			aggregation: Aggregation::AllOrNothing,
			..item("a", 3)
		}];
		let p = policy(GradingConfig::default());
		assert_eq!(
			grade(student(vec![result("a", 9, 10)]), &items, &p).final_grade(),
			Some(0.0)
		);
		assert_eq!(
			grade(student(vec![result("a", 10, 10)]), &items, &p).final_grade(),
			Some(100.0)
		);
	}

	#[test]
	fn test_not_submitted_is_withheld_by_default_and_zero_by_policy() {
		let items = [item("a", 1)];
		for (state, reason) in [
			(SubmissionOutcome::NotSubmitted, Reason::NotSubmitted),
			(SubmissionOutcome::SubmittedEmpty, Reason::SubmittedEmpty),
		] {
			let absent = StudentReport::new("s", state);
			let g = grade(absent.clone(), &items, &policy(GradingConfig::default()));
			assert_eq!(withheld_reason(&g), Some(reason));
			assert_eq!(g.final_grade(), None);

			// A policy zero is a real 0 that says why, and no curve lifts it.
			let zero = policy(GradingConfig {
				missing: MissingPolicy::Zero,
				curve: Curve::Template {
					name: CurveTemplate::Linear,
					lower: 60.0,
					upper: 100.0,
				},
				..GradingConfig::default()
			});
			let g = grade(absent, &items, &zero);
			assert_eq!(g.final_grade(), Some(0.0));
			assert_eq!(g.reason(), Some(reason));
		}
	}

	#[test]
	fn test_excused_is_always_withheld() {
		let zero = policy(GradingConfig {
			missing: MissingPolicy::Zero,
			..GradingConfig::default()
		});
		for state in [
			SubmissionOutcome::NotSubmitted,
			SubmissionOutcome::Executable,
		] {
			let report = StudentReport {
				excused: true,
				test_results: vec![result("a", 1, 1)],
				..StudentReport::new("s", state)
			};
			let g = grade(report, &[item("a", 1)], &zero);
			assert_eq!(withheld_reason(&g), Some(Reason::Excused));
		}
	}

	#[test]
	fn test_unmatched_submission_is_pending_review() {
		let report = StudentReport {
			test_results: vec![result("a", 1, 1)],
			..StudentReport::new("s", SubmissionOutcome::ReceivedUnmatched)
		};
		let g = grade(report, &[item("a", 1)], &policy(GradingConfig::default()));
		assert_eq!(withheld_reason(&g), Some(Reason::PendingReview));
	}

	#[test]
	fn test_a_task_failure_is_withheld() {
		let report = StudentReport {
			error: Some("grading task failed: panicked".into()),
			..StudentReport::new("s", SubmissionOutcome::Executable)
		};
		let g = grade(report, &[item("a", 1)], &policy(GradingConfig::default()));
		assert_eq!(withheld_reason(&g), Some(Reason::GradingTaskFailed));
	}

	fn no_file(item: &str) -> TestResult {
		TestResult {
			item_id: item.into(),
			file: None,
			cases: vec![CaseResult {
				status: TestStatus::Missing,
				..case("c0", Some(Fault::Student), Some(Cause::NoFile))
			}],
		}
	}

	#[test]
	fn test_a_missing_file_is_withheld_by_default_and_zero_by_policy() {
		let items = [item("a", 1), item("b", 1)];
		let report = student(vec![no_file("a"), result("b", 1, 1)]);
		let g = grade(report.clone(), &items, &policy(GradingConfig::default()));
		assert_eq!(withheld_reason(&g), Some(Reason::MissingFile));

		let zero = policy(GradingConfig {
			missing_file: MissingPolicy::Zero,
			..GradingConfig::default()
		});
		let g = grade(report, &items, &zero);
		assert_eq!(g.final_grade(), Some(50.0));
		assert_eq!(
			g.items[0].outcome,
			ItemOutcome::Graded {
				score: 0.0,
				reason: Some(Reason::MissingFile)
			}
		);

		// With nothing handed in at all, it is a policy zero: no curve lifts it.
		let curved_zero = policy(GradingConfig {
			missing_file: MissingPolicy::Zero,
			curve: Curve::Template {
				name: CurveTemplate::Linear,
				lower: 60.0,
				upper: 100.0,
			},
			..GradingConfig::default()
		});
		let g = grade(
			student(vec![no_file("a"), no_file("b")]),
			&items,
			&curved_zero,
		);
		assert_eq!(
			g.outcome,
			GradeOutcome::Graded {
				score: 0.0,
				raw_grade: 0.0,
				final_grade: 0.0,
				reason: Some(Reason::MissingFile),
			}
		);
	}

	#[test]
	fn test_a_teacher_or_environment_fault_withholds_that_student_only() {
		let items = [item("a", 1)];
		for (fault, cause, reason) in [
			(Fault::Teacher, Cause::TeacherImport, Reason::TeacherFault),
			(Fault::Environment, Cause::Spawn, Reason::EnvironmentFault),
		] {
			let broken = student(vec![TestResult {
				item_id: "a".into(),
				file: None,
				cases: vec![
					case("ok", None, None),
					case("bad", Some(fault), Some(cause)),
				],
			}]);
			let fine = student(vec![result("a", 1, 1)]);
			let mut reports = [broken, fine];
			grade_all(&mut reports, &items, &policy(GradingConfig::default())).unwrap();

			let g = reports[0].grade.as_ref().unwrap();
			assert_eq!(withheld_reason(g), Some(reason));
			assert_eq!(
				g.items[0].outcome,
				ItemOutcome::Withheld {
					reason,
					blocking_case: Some("bad".into()),
					blocking_cause: Some(cause),
				}
			);
			assert_eq!(reports[1].final_grade(), Some(100.0));
		}
	}

	#[test]
	fn test_a_formula_that_does_not_compile_refuses_the_batch() {
		let err = Policy::compile(
			GradingConfig {
				curve: Curve::Formula {
					formula: "undefined_var + 1".into(),
				},
				..GradingConfig::default()
			},
			false,
		)
		.err()
		.expect("refused");
		assert!(err.to_string().contains("does not compile"), "{err}");
	}

	#[test]
	fn test_a_formula_failing_on_one_student_withholds_only_that_student() {
		let p = curve(Curve::Formula {
			formula: "if score < 0.5 { throw \"no\" } else { fraction * scale }".into(),
		});
		let items = [item("a", 1)];
		let mut reports = [
			student(vec![result("a", 0, 1)]),
			student(vec![result("a", 1, 1)]),
		];
		grade_all(&mut reports, &items, &p).unwrap();
		let failed = reports[0].grade.as_ref().unwrap();
		assert_eq!(withheld_reason(failed), Some(Reason::FormulaError));
		assert_eq!(reports[1].final_grade(), Some(100.0));
	}

	#[test]
	fn test_a_formula_out_of_range_is_withheld_not_clamped() {
		for formula in ["1.0 / 0.0", "scale + 1.0", "-1", "true"] {
			let p = curve(Curve::Formula {
				formula: formula.into(),
			});
			let g = grade(student(vec![result("a", 1, 1)]), &[item("a", 1)], &p);
			assert_eq!(withheld_reason(&g), Some(Reason::FormulaError), "{formula}");
		}
		let p = curve(Curve::Formula {
			formula: "score * 50".into(),
		});
		let g = grade(student(vec![result("a", 1, 1)]), &[item("a", 1)], &p);
		assert_eq!(g.final_grade(), Some(50.0), "an int result is a number");
	}

	#[test]
	fn test_a_curve_keeps_the_raw_grade_beside_it() {
		let p = curve(Curve::Template {
			name: CurveTemplate::Sqrt,
			lower: 60.0,
			upper: 100.0,
		});
		let g = grade(student(vec![result("a", 1, 4)]), &[item("a", 1)], &p);
		assert_eq!(
			g.outcome,
			GradeOutcome::Graded {
				score: 0.25,
				raw_grade: 25.0,
				final_grade: 80.0,
				reason: None,
			}
		);
	}

	#[test]
	fn test_templates_map_the_fraction() {
		let at = |name, passed| {
			let p = curve(Curve::Template {
				name,
				lower: 60.0,
				upper: 100.0,
			});
			grade(student(vec![result("a", passed, 10)]), &[item("a", 1)], &p)
				.final_grade()
				.unwrap()
		};
		assert_eq!(at(CurveTemplate::Linear, 10), 100.0);
		assert_eq!(at(CurveTemplate::Linear, 0), 60.0);
		assert_eq!(at(CurveTemplate::Log, 10), 100.0);
		assert_eq!(at(CurveTemplate::Strict, 10), 100.0);
		assert_eq!(at(CurveTemplate::Strict, 9), 80.0);
		assert_eq!(at(CurveTemplate::Strict, 5), 60.0);
	}

	#[test]
	fn test_rounding_is_half_away_from_zero() {
		let g = grade(
			student(vec![result("a", 2, 3)]),
			&[item("a", 1)],
			&policy(GradingConfig::default()),
		);
		assert_eq!(g.final_grade(), Some(66.67));
		let whole = policy(GradingConfig {
			decimals: 0,
			..GradingConfig::default()
		});
		let g = grade(student(vec![result("a", 2, 3)]), &[item("a", 1)], &whole);
		assert_eq!(g.final_grade(), Some(67.0));
		assert_eq!(round_half_away(0.125, 2), 0.13);
		assert_eq!(round_half_away(2.5, 0), 3.0);
	}

	#[test]
	fn test_grading_is_idempotent_and_order_independent() {
		let items = [item("a", 2), item("b", 1)];
		let p = policy(GradingConfig::default());
		let reports = || {
			vec![
				StudentReport {
					student_id: "x".into(),
					..student(vec![result("b", 0, 2), result("a", 3, 7)])
				},
				StudentReport {
					student_id: "y".into(),
					..student(vec![result("a", 7, 7), result("b", 2, 2)])
				},
			]
		};
		let mut once = reports();
		grade_all(&mut once, &items, &p).unwrap();
		let mut twice = once.clone();
		grade_all(&mut twice, &items, &p).unwrap();
		let mut reversed: Vec<_> = reports().into_iter().rev().collect();
		grade_all(&mut reversed, &items, &p).unwrap();
		reversed.reverse();
		let json = |r: &[StudentReport]| serde_json::to_string(r).unwrap();
		assert_eq!(json(&once), json(&twice));
		assert_eq!(json(&once), json(&reversed));
	}

	#[test]
	fn test_lint_counts_only_when_declared() {
		let items = [item("a", 1)];
		let linted = StudentReport {
			lint: Some(LintOutcome::Scored { score: 50.0 }),
			..student(vec![result("a", 1, 1)])
		};
		let g = grade(linted.clone(), &items, &policy(GradingConfig::default()));
		assert_eq!(
			g.final_grade(),
			Some(100.0),
			"lint is ignored unless declared"
		);

		let with_lint = policy(GradingConfig {
			lint_points: Some(1),
			..GradingConfig::default()
		});
		assert_eq!(grade(linted, &items, &with_lint).final_grade(), Some(75.0));

		let failed = StudentReport {
			lint: Some(LintOutcome::Failed {
				message: "exited with 2".into(),
			}),
			..student(vec![result("a", 1, 1)])
		};
		let g = grade(failed, &items, &with_lint);
		assert_eq!(withheld_reason(&g), Some(Reason::LintFailed));
	}

	#[test]
	fn test_lint_on_a_missing_file_follows_the_missing_file_policy() {
		let items = [item("a", 1), item("b", 1)];
		let report = StudentReport {
			lint: Some(LintOutcome::NoFile),
			..student(vec![no_file("a"), result("b", 1, 1)])
		};
		let default = policy(GradingConfig {
			lint_points: Some(1),
			..GradingConfig::default()
		});
		let g = grade(report.clone(), &items, &default);
		assert_eq!(withheld_reason(&g), Some(Reason::MissingFile));

		let zero = policy(GradingConfig {
			lint_points: Some(1),
			missing_file: MissingPolicy::Zero,
			..GradingConfig::default()
		});
		// 1 of 3 points: neither the missing item nor its lint earns anything.
		assert_eq!(grade(report, &items, &zero).final_grade(), Some(33.33));
	}

	#[test]
	fn test_evidence_that_breaks_the_runners_contract_is_an_error() {
		let p = policy(GradingConfig::default());
		let mut missing = [student(vec![])];
		assert!(grade_all(&mut missing, &[item("a", 1)], &p).is_err());

		let unowned = student(vec![TestResult {
			item_id: "a".into(),
			file: None,
			cases: vec![CaseResult {
				status: TestStatus::Failed,
				..case("c", None, None)
			}],
		}]);
		assert!(grade_all(&mut [unowned], &[item("a", 1)], &p).is_err());
		assert!(
			grade_all(&mut [], &[item("a", 0)], &p).is_err(),
			"0 points in total"
		);
	}

	#[test]
	fn test_policy_bounds_are_refused() {
		for config in [
			GradingConfig {
				scale: 0.0,
				..GradingConfig::default()
			},
			GradingConfig {
				scale: f64::NAN,
				..GradingConfig::default()
			},
			GradingConfig {
				decimals: 5,
				..GradingConfig::default()
			},
			GradingConfig {
				curve: Curve::Template {
					name: CurveTemplate::Linear,
					lower: 90.0,
					upper: 60.0,
				},
				..GradingConfig::default()
			},
			GradingConfig {
				curve: Curve::Template {
					name: CurveTemplate::Linear,
					lower: 0.0,
					upper: 101.0,
				},
				..GradingConfig::default()
			},
		] {
			assert!(
				Policy::compile(config.clone(), false).is_err(),
				"{config:?}"
			);
		}
	}

	#[test]
	fn test_diagnostics_name_a_broken_checker_and_a_wrong_file_pattern() {
		let checker = |id: &str| TestResult {
			item_id: id.into(),
			file: None,
			cases: vec![case("c", Some(Fault::Student), Some(Cause::Checker))],
		};
		let reports = [
			student(vec![checker("a"), no_file("b")]),
			student(vec![checker("a"), no_file("b")]),
			StudentReport::new("absent", SubmissionOutcome::NotSubmitted),
		];
		let warnings = diagnostics(&reports, &[item("a", 1), item("b", 1)]);
		assert_eq!(warnings.len(), 2, "{warnings:?}");
		assert!(warnings[0].contains("'a'") && warnings[0].contains("checker"));
		assert!(warnings[1].contains("'b'") && warnings[1].contains("[meta] file"));

		let one_right = [
			student(vec![checker("a")]),
			student(vec![result("a", 1, 1)]),
		];
		assert!(diagnostics(&one_right, &[item("a", 1)]).is_empty());
	}
}
