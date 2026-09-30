//! Grades leaving ScriptMark: a CSV a teacher can read, and the set pushed to Canvas.
//! Both go by a grade's state, never by its number — a withheld grade is an empty cell and
//! is never pushed; a real zero is `0` and is.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{Result, bail};

use crate::models::{GradeOutcome, GradingItem, ItemOutcome, Reason, StudentReport};

/// A snake_case word for a serialisable value, as the JSON results write it.
pub fn word<T: serde::Serialize>(value: &T) -> String {
	serde_json::to_value(value)
		.ok()
		.and_then(|v| v.as_str().map(str::to_string))
		.unwrap_or_default()
}

/// What `grades push` sends, and who it leaves out and why.
#[derive(Debug, Default, PartialEq)]
pub struct PushSet {
	/// Canvas user id → grade.
	pub grades: BTreeMap<u64, f64>,
	/// Why a student was left out → how many.
	pub skipped: BTreeMap<String, usize>,
}

/// Choose the grades to push: every graded student with a Canvas user id, zeros included.
///
/// Refuses unscored results, and two reports claiming one Canvas user — one of them would
/// silently overwrite the other.
pub fn grades_to_push(reports: &[StudentReport]) -> Result<PushSet> {
	let mut set = PushSet::default();
	let mut skip = |why: String| *set.skipped.entry(why).or_default() += 1;
	for report in reports {
		let Some(grade) = &report.grade else {
			bail!(
				"{} has no grade: these are `run` results; score them with `grade`",
				report.student_id
			);
		};
		match (&grade.outcome, report.canvas_user_id) {
			(GradeOutcome::Withheld { reason, .. }, _) => {
				skip(format!("withheld: {}", word(reason)))
			}
			(GradeOutcome::Graded { .. }, None) => skip("no Canvas user id".into()),
			(GradeOutcome::Graded { final_grade, .. }, Some(uid)) => {
				if set.grades.insert(uid, *final_grade).is_some() {
					bail!("two students in these results are Canvas user {uid}; refusing to push");
				}
			}
		}
	}
	Ok(set)
}

/// One row per student: the grade, why there is none or why it is a policy zero, and each
/// item's score and state. Withheld cells are empty; a zero is `0`.
pub fn write_grades_csv<W: Write>(
	reports: &[StudentReport],
	items: &[GradingItem],
	out: W,
) -> Result<()> {
	let mut csv = csv::Writer::from_writer(out);
	let mut header: Vec<String> = [
		"student_id",
		"student_name",
		"canvas_user_id",
		"grade",
		"reason",
		"score",
		"max",
		"raw_grade",
		"final_grade",
	]
	.map(String::from)
	.to_vec();
	for item in items {
		header.extend([
			format!("{}_score", item.id),
			format!("{}_state", item.id),
			format!("{}_reason", item.id),
		]);
	}
	csv.write_record(&header)?;

	for report in reports {
		let Some(grade) = &report.grade else {
			bail!(
				"{} has no grade: these are `run` results; score them with `grade`",
				report.student_id
			);
		};
		let grade_cell = |x: f64| number(x, grade.basis.decimals);
		let points_cell = |x: f64| number(x, POINTS_DECIMALS);
		let reason = |r: Option<Reason>| r.map(|r| word(&r)).unwrap_or_default();
		let (state, score, raw, fin) = match grade.outcome {
			GradeOutcome::Graded {
				score,
				raw_grade,
				final_grade,
				..
			} => (
				"graded",
				points_cell(score),
				grade_cell(raw_grade),
				grade_cell(final_grade),
			),
			GradeOutcome::Withheld { .. } => {
				("withheld", String::new(), String::new(), String::new())
			}
		};
		let mut row = vec![
			report.student_id.clone(),
			report.student_name.clone().unwrap_or_default(),
			report
				.canvas_user_id
				.map(|u| u.to_string())
				.unwrap_or_default(),
			state.to_string(),
			reason(grade.reason()),
			score,
			points_cell(grade.max),
			raw,
			fin,
		];
		for item in items {
			match grade.items.iter().find(|s| s.item_id == item.id) {
				Some(scored) => match &scored.outcome {
					ItemOutcome::Graded { score, reason: r } => {
						row.extend([points_cell(*score), "graded".into(), reason(*r)])
					}
					ItemOutcome::Withheld { reason: r, .. } => {
						row.extend([String::new(), "withheld".into(), reason(Some(*r))])
					}
				},
				// The student was withheld before any item was looked at.
				None => row.extend([String::new(), String::new(), String::new()]),
			}
		}
		csv.write_record(&row)?;
	}
	csv.flush()?;
	Ok(())
}

/// Places points are shown to: finer than any grade, so item scores still add up.
pub const POINTS_DECIMALS: u8 = 4;

/// A number rounded half away from zero, as grades are, without trailing zeros: `87.5`,
/// `0`, `66.67`.
pub fn number(x: f64, decimals: u8) -> String {
	let s = format!(
		"{:.*}",
		usize::from(decimals),
		crate::grading::round_half_away(x, decimals)
	);
	let s = if s.contains('.') {
		s.trim_end_matches('0').trim_end_matches('.')
	} else {
		&s
	};
	if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_number_rounds_half_away_and_trims() {
		assert_eq!(number(86.5, 0), "87");
		assert_eq!(number(2.0 / 3.0, 2), "0.67");
		assert_eq!(number(2.0 / 3.0, POINTS_DECIMALS), "0.6667");
		assert_eq!(number(100.0, 2), "100");
		assert_eq!(number(0.0, 2), "0");
		assert_eq!(number(-0.0001, 2), "0");
	}

	use crate::models::fixtures::{graded, withheld};
	use crate::models::{Grade, SubmissionOutcome};

	fn with_canvas(mut report: StudentReport, uid: u64) -> StudentReport {
		report.canvas_user_id = Some(uid);
		report
	}

	#[test]
	fn test_push_sends_zeros_and_skips_withheld() {
		let reports = [
			with_canvas(graded("alice", 87.5), 1),
			with_canvas(graded("bob", 0.0), 2),
			with_canvas(
				withheld("carol", SubmissionOutcome::Executable, Reason::TeacherFault),
				3,
			),
			with_canvas(
				withheld("dan", SubmissionOutcome::NotSubmitted, Reason::Excused),
				4,
			),
			graded("erin", 90.0),
		];
		let set = grades_to_push(&reports).unwrap();
		assert_eq!(set.grades, BTreeMap::from([(1, 87.5), (2, 0.0)]));
		assert_eq!(
			set.skipped,
			BTreeMap::from([
				("no Canvas user id".to_string(), 1),
				("withheld: excused".to_string(), 1),
				("withheld: teacher_fault".to_string(), 1),
			])
		);
	}

	#[test]
	fn test_push_refuses_unscored_results_and_a_shared_canvas_user() {
		let unscored = [StudentReport::new("alice", SubmissionOutcome::Executable)];
		assert!(grades_to_push(&unscored).is_err());
		let shared = [
			with_canvas(graded("a", 1.0), 7),
			with_canvas(graded("b", 2.0), 7),
		];
		assert!(grades_to_push(&shared).is_err());
	}

	#[test]
	fn test_grades_csv_leaves_withheld_empty_and_writes_zero_with_its_reason() {
		let items = [GradingItem::new("q1")];
		let mut policy_zero = graded("bob", 0.0);
		if let Some(Grade {
			outcome: GradeOutcome::Graded { reason, .. },
			..
		}) = &mut policy_zero.grade
		{
			*reason = Some(Reason::NotSubmitted);
		}
		let reports = [
			graded("alice", 87.5),
			policy_zero,
			withheld(
				"carol",
				SubmissionOutcome::Executable,
				Reason::EnvironmentFault,
			),
		];
		let mut out = Vec::new();
		write_grades_csv(&reports, &items, &mut out).unwrap();
		let text = String::from_utf8(out).unwrap();
		let lines: Vec<&str> = text.lines().collect();
		assert_eq!(
			lines[0],
			"student_id,student_name,canvas_user_id,grade,reason,score,max,raw_grade,final_grade,\
			 q1_score,q1_state,q1_reason"
		);
		assert_eq!(lines[1], "alice,,,graded,,0.875,1,87.5,87.5,,,");
		assert_eq!(lines[2], "bob,,,graded,not_submitted,0,1,0,0,,,");
		assert_eq!(lines[3], "carol,,,withheld,environment_fault,,1,,,,,");
	}
}
