//! Grades leaving ScriptMark: grade sheets a teacher can read, and the set pushed to Canvas.
//! Both go by a grade's state, never by its number — a withheld grade is an empty cell and
//! is never pushed; a real zero is `0` and is.
//!
//! A sheet is built once as a [`Table`] of typed cells and only then written, as CSV or as
//! XLSX, so the two cannot disagree: identifiers and words are text, so a 学号 keeps its
//! leading zeros, and scores are numbers rounded as the CSV prints them.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rust_xlsxwriter::{Format as Style, FormatBorder, IgnoreError, Workbook};
use serde_json::Value;

use crate::grading::{self, round_half_away};
use crate::models::{GradeOutcome, GradingItem, ItemOutcome, Reason, StudentReport};
use crate::record::{Record, Source};

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
				"{} has no grade: the record has no score revision; score it with `scriptmark rescore`",
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

/// What a grade sheet is written as, by the output file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
	/// The grades alone.
	Csv,
	/// The grades, and the items, cases and record behind them on sheets of their own.
	Xlsx,
}

impl Format {
	pub fn of(path: &Path) -> Result<Format> {
		let extension = path
			.extension()
			.and_then(|e| e.to_str())
			.map(str::to_ascii_lowercase);
		match extension.as_deref() {
			Some("csv") => Ok(Format::Csv),
			Some("xlsx") => Ok(Format::Xlsx),
			_ => bail!(
				"{} is not a .csv or .xlsx file: the grade sheet's extension says which to write",
				path.display()
			),
		}
	}
}

/// Revision `n` of `record` as a grade sheet in `format`.
pub fn sheet(record: &Record, n: u32, format: Format) -> Result<Vec<u8>> {
	match format {
		Format::Csv => {
			let mut out = Vec::new();
			write_csv(&grades(record, n)?, &mut out)?;
			Ok(out)
		}
		Format::Xlsx => workbook(&[
			grades(record, n)?,
			items(record, n)?,
			cases(&record.view(Some(n))?.reports),
			about(record, n)?,
		]),
	}
}

/// One cell of a sheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
	Empty,
	Text(String),
	/// Rounded to `decimals` places, half away from zero; CSV prints it with [`number`].
	Number {
		value: f64,
		decimals: u8,
	},
}

/// What one spreadsheet cell holds, in UTF-16 units: Excel's limit.
const CELL_LIMIT: usize = 32_767;

/// Where a text too long for one cell was cut. The record keeps all of it.
const CUT: &str = " … (cut)";

impl Cell {
	/// Text, cut to what one spreadsheet cell holds.
	pub fn text(text: impl Into<String>) -> Cell {
		let text = text.into();
		if text.encode_utf16().count() <= CELL_LIMIT {
			return Cell::Text(text);
		}
		let room = CELL_LIMIT - CUT.encode_utf16().count();
		let mut used = 0;
		let end = text
			.char_indices()
			.find(|(_, c)| {
				used += c.len_utf16();
				used > room
			})
			.map_or(text.len(), |(i, _)| i);
		Cell::Text(format!("{}{CUT}", &text[..end]))
	}

	pub fn number(x: f64, decimals: u8) -> Cell {
		let value = round_half_away(x, decimals);
		Cell::Number {
			// `-0` would read as a deduction that is not there.
			value: if value == 0.0 { 0.0 } else { value },
			decimals,
		}
	}

	fn maybe(text: Option<impl Into<String>>) -> Cell {
		text.map_or(Cell::Empty, Cell::text)
	}

	/// A snake_case word, as the record writes it.
	fn word<T: serde::Serialize>(value: Option<T>) -> Cell {
		Cell::maybe(value.map(|v| word(&v)).filter(|w| !w.is_empty()))
	}

	/// What CSV writes.
	pub fn csv(&self) -> String {
		match self {
			Cell::Empty => String::new(),
			Cell::Text(text) => text.clone(),
			Cell::Number { value, decimals } => number(*value, *decimals),
		}
	}
}

/// A sheet: a header, and rows of cells under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
	pub name: &'static str,
	pub header: Vec<String>,
	pub rows: Vec<Vec<Cell>>,
	/// Leading columns that say whose a row is, kept in view while scrolling.
	pub keys: u16,
}

fn header(names: &[&str]) -> Vec<String> {
	names.iter().map(|n| n.to_string()).collect()
}

/// Which grading a row is from. Written on every row of the grades, so that a row copied
/// into another sheet still says.
struct Origin<'a> {
	assignment: &'a str,
	revision: u32,
	/// The evidence digest's first 12 characters, as `grades push` prints it.
	evidence: &'a str,
}

/// One row per student, in the record's order, as revision `n` scored them: the grade, why
/// there is none or why it is a policy zero, each item's score and state, what lint earned
/// when the policy counts it, and which grading this is.
pub fn grades(record: &Record, n: u32) -> Result<Table> {
	let revision = record.revision(n)?;
	grade_table(
		&record.view(Some(n))?.reports,
		&revision.policy.items,
		revision.policy.grading.lint_points,
		&Origin {
			assignment: &record.evidence.assignment.name,
			revision: n,
			evidence: &record.digest[..12],
		},
	)
}

fn grade_table(
	reports: &[StudentReport],
	items: &[GradingItem],
	lint_points: Option<u32>,
	origin: &Origin,
) -> Result<Table> {
	let mut header = header(&[
		"student_id",
		"student_name",
		"canvas_user_id",
		"state",
		"reason",
		"score",
		"max",
		"raw_grade",
		"final_grade",
	]);
	for item in items {
		header.extend([
			format!("{}_score", item.id),
			format!("{}_state", item.id),
			format!("{}_reason", item.id),
		]);
	}
	if lint_points.is_some() {
		header.push("lint".into());
	}
	header.extend(["assignment", "revision", "evidence"].map(String::from));

	let points = |x: f64| Cell::number(x, POINTS_DECIMALS);
	let reason = |r: Option<Reason>| Cell::word(r);
	let mut rows = Vec::with_capacity(reports.len());
	for report in reports {
		let Some(grade) = &report.grade else {
			bail!(
				"{} has no grade: the record has no score revision; score it with `scriptmark rescore`",
				report.student_id
			);
		};
		let (state, score, raw, fin) = match grade.outcome {
			GradeOutcome::Graded {
				score,
				raw_grade,
				final_grade,
				..
			} => (
				"graded",
				points(score),
				Cell::number(raw_grade, grade.basis.decimals),
				Cell::number(final_grade, grade.basis.decimals),
			),
			GradeOutcome::Withheld { .. } => ("withheld", Cell::Empty, Cell::Empty, Cell::Empty),
		};
		let mut row = vec![
			Cell::text(&report.student_id),
			Cell::maybe(report.student_name.as_deref()),
			Cell::maybe(report.canvas_user_id.map(|u| u.to_string())),
			Cell::text(state),
			reason(grade.reason()),
			score,
			points(grade.max),
			raw,
			fin,
		];
		for item in items {
			match grade.items.iter().find(|s| s.item_id == item.id) {
				Some(scored) => match &scored.outcome {
					ItemOutcome::Graded { score, reason: r } => {
						row.extend([points(*score), Cell::text("graded"), reason(*r)])
					}
					ItemOutcome::Withheld { reason: r, .. } => {
						row.extend([Cell::Empty, Cell::text("withheld"), reason(Some(*r))])
					}
				},
				// The student was withheld before any item was looked at.
				None => row.extend([Cell::Empty, Cell::Empty, Cell::Empty]),
			}
		}
		if let Some(of) = lint_points {
			// Lint counts only where the evidence was scored.
			let earned = grading::gate(report)
				.is_none()
				.then(|| grading::lint_earned(of, report.lint.as_ref()))
				.flatten();
			row.push(earned.map_or(Cell::Empty, points));
		}
		row.extend([
			Cell::text(origin.assignment),
			Cell::number(f64::from(origin.revision), 0),
			Cell::text(origin.evidence),
		]);
		rows.push(row);
	}
	Ok(Table {
		name: "grades",
		header,
		rows,
		keys: 2,
	})
}

/// What each item column is: the items revision `n` scored, in declaration order, and lint
/// when the policy counts it.
pub fn items(record: &Record, n: u32) -> Result<Table> {
	let policy = &record.revision(n)?.policy;
	let mut rows: Vec<Vec<Cell>> = policy
		.items
		.iter()
		.map(|item| {
			vec![
				Cell::text(&item.id),
				Cell::maybe(item.title.as_deref()),
				Cell::number(f64::from(item.points), 0),
				Cell::word(Some(item.aggregation)),
			]
		})
		.collect();
	if let Some(points) = policy.grading.lint_points {
		rows.push(vec![
			Cell::text("lint"),
			Cell::Empty,
			Cell::number(f64::from(points), 0),
			Cell::Empty,
		]);
	}
	Ok(Table {
		name: "items",
		header: header(&["item", "title", "points", "aggregation"]),
		rows,
		keys: 1,
	})
}

/// The evidence behind the grades: one row per test case, and one for a student who has
/// none, saying why — so every student appears.
pub fn cases(reports: &[StudentReport]) -> Table {
	let mut rows = Vec::new();
	for report in reports {
		let student = || {
			[
				Cell::text(&report.student_id),
				Cell::maybe(report.student_name.as_deref()),
				Cell::word(Some(report.submission_state)),
			]
		};
		let before = rows.len();
		for result in &report.test_results {
			for case in &result.cases {
				let mut row = student().to_vec();
				row.extend([
					Cell::text(&result.item_id),
					Cell::text(&case.case_name),
					Cell::word(Some(case.status)),
					Cell::maybe(case.actual.as_deref()),
					Cell::maybe(case.expected.as_deref()),
					Cell::maybe(case.failure.as_ref().map(|f| f.message.as_str())),
					case.elapsed_ms
						.map_or(Cell::Empty, |ms| Cell::number(ms as f64, 0)),
					Cell::word(case.fault),
					Cell::word(case.cause),
				]);
				rows.push(row);
			}
		}
		if rows.len() == before {
			let (status, why) = match &report.error {
				Some(error) => ("error".to_string(), Cell::text(error)),
				None => (
					word(&report.status()),
					Cell::word(report.grade.as_ref().and_then(|g| g.reason())),
				),
			};
			let mut row = student().to_vec();
			row.extend([
				Cell::Empty,
				Cell::Empty,
				Cell::text(status),
				Cell::Empty,
				Cell::Empty,
				why,
				Cell::Empty,
				Cell::Empty,
				Cell::Empty,
			]);
			rows.push(row);
		}
	}
	Table {
		name: "cases",
		header: header(&[
			"student_id",
			"student_name",
			"submission_state",
			"item_id",
			"case_name",
			"status",
			"actual",
			"expected",
			"message",
			"elapsed_ms",
			"fault",
			"cause",
		]),
		rows,
		keys: 2,
	}
}

/// Which grading a workbook is: the assignment, the record and revision, the inputs and
/// builds, and the policy the revision was scored under.
pub fn about(record: &Record, n: u32) -> Result<Table> {
	let revision = record.revision(n)?;
	let evidence = &record.evidence;
	let path = |p: &Path| Cell::text(p.display().to_string());
	let count = |x: usize| Cell::number(x as f64, 0);
	let mut rows: Vec<(String, Cell)> = vec![
		("assignment".into(), Cell::text(&evidence.assignment.name)),
		(
			"canvas_course_id".into(),
			Cell::maybe(evidence.assignment.canvas_course_id.map(|i| i.to_string())),
		),
		(
			"canvas_assignment_id".into(),
			Cell::maybe(
				evidence
					.assignment
					.canvas_assignment_id
					.map(|i| i.to_string()),
			),
		),
		("revision".into(), Cell::number(f64::from(n), 0)),
		("revisions".into(), count(record.revisions.len())),
		("evidence".into(), Cell::text(&record.digest)),
		("revision_checksum".into(), Cell::text(&revision.checksum)),
		("students".into(), count(evidence.students.len())),
		("tests".into(), path(&evidence.inputs.tests)),
		(
			"assignment_toml".into(),
			Cell::maybe(
				evidence
					.inputs
					.assignment
					.as_ref()
					.map(|p| p.display().to_string()),
			),
		),
	];
	match &evidence.inputs.source {
		Source::Local { dirs, roster } => {
			let dirs: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
			rows.push(("submissions".into(), Cell::text(dirs.join("\n"))));
			rows.push((
				"roster".into(),
				Cell::maybe(roster.as_ref().map(|p| p.display().to_string())),
			));
		}
		Source::Canvas { bundle } => rows.push(("canvas_bundle".into(), path(bundle))),
	}
	rows.extend([
		("run_by".into(), Cell::text(&evidence.scriptmark)),
		("scored_by".into(), Cell::text(&revision.scriptmark)),
		(
			"derived_items".into(),
			Cell::text(revision.policy.derived_items.to_string()),
		),
	]);
	// The policy, as `[grading]` spells it.
	let grading = serde_json::to_value(&revision.policy.grading)
		.context("the grading policy is plain JSON")?;
	for (key, value) in grading.as_object().into_iter().flatten() {
		let cell = match value {
			Value::Null => Cell::Empty,
			Value::String(s) => Cell::text(s),
			other => Cell::text(other.to_string()),
		};
		rows.push((format!("grading.{key}"), cell));
	}
	Ok(Table {
		name: "record",
		header: header(&["field", "value"]),
		rows: rows
			.into_iter()
			.map(|(field, value)| vec![Cell::Text(field), value])
			.collect(),
		keys: 1,
	})
}

/// Write `table` as CSV, after a UTF-8 byte order mark: without one, spreadsheet software
/// reads Chinese names in some other encoding.
pub fn write_csv<W: Write>(table: &Table, mut out: W) -> Result<()> {
	out.write_all("\u{feff}".as_bytes())?;
	let mut csv = csv::Writer::from_writer(out);
	csv.write_record(&table.header)?;
	for row in &table.rows {
		csv.write_record(row.iter().map(Cell::csv))?;
	}
	csv.flush()?;
	Ok(())
}

/// `tables` as one workbook, a sheet each, with a bold header kept in view, a filter on it
/// and the columns that say whose a row is frozen.
pub fn workbook(tables: &[Table]) -> Result<Vec<u8>> {
	let mut workbook = Workbook::new();
	let bold = Style::new()
		.set_bold()
		.set_border_bottom(FormatBorder::Thin);
	for table in tables {
		let sheet = workbook.add_worksheet();
		sheet.set_name(table.name)?;
		let last_col = u16::try_from(table.header.len().saturating_sub(1))
			.context("too many columns for a spreadsheet")?;
		let last_row =
			u32::try_from(table.rows.len()).context("too many rows for a spreadsheet")?;
		for (col, title) in (0..).zip(&table.header) {
			sheet.write_string_with_format(0, col, title, &bold)?;
		}
		for (row, cells) in (1..).zip(&table.rows) {
			for (col, cell) in (0..).zip(cells) {
				match cell {
					Cell::Empty => {}
					Cell::Text(text) => {
						sheet.write_string(row, col, text)?;
					}
					Cell::Number { value, .. } => {
						sheet.write_number(row, col, *value)?;
					}
				}
			}
		}
		sheet.set_freeze_panes(1, table.keys)?;
		sheet.autofilter(0, 0, last_row, last_col)?;
		if last_row > 0 {
			// A 学号 is text on purpose; Excel would flag every one.
			sheet.ignore_error_range(1, 0, last_row, last_col, IgnoreError::NumberStoredAsText)?;
		}
		sheet.set_autofit_max_width(320).autofit();
	}
	Ok(workbook.save_to_buffer()?)
}

/// Places points are shown to: finer than any grade, so item scores still add up.
pub const POINTS_DECIMALS: u8 = 4;

/// A number rounded half away from zero, as grades are, without trailing zeros: `87.5`,
/// `0`, `66.67`.
pub fn number(x: f64, decimals: u8) -> String {
	let s = format!("{:.*}", usize::from(decimals), round_half_away(x, decimals));
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
	use crate::models::{Aggregation, Grade, ItemScore, LintOutcome, SubmissionOutcome};

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

	const ORIGIN: Origin = Origin {
		assignment: "hw",
		revision: 2,
		evidence: "0123456789ab",
	};

	fn csv(table: &Table) -> String {
		let mut out = Vec::new();
		write_csv(table, &mut out).unwrap();
		String::from_utf8(out).unwrap()
	}

	#[test]
	fn test_grades_leave_withheld_empty_and_write_zero_with_its_reason() {
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
		let text = csv(&grade_table(&reports, &items, None, &ORIGIN).unwrap());
		let text = text.strip_prefix('\u{feff}').expect("a byte order mark");
		let lines: Vec<&str> = text.lines().collect();
		assert_eq!(
			lines[0],
			"student_id,student_name,canvas_user_id,state,reason,score,max,raw_grade,final_grade,\
			 q1_score,q1_state,q1_reason,assignment,revision,evidence"
		);
		assert_eq!(
			lines[1],
			"alice,,,graded,,0.875,1,87.5,87.5,,,,hw,2,0123456789ab"
		);
		assert_eq!(
			lines[2],
			"bob,,,graded,not_submitted,0,1,0,0,,,,hw,2,0123456789ab"
		);
		assert_eq!(
			lines[3],
			"carol,,,withheld,environment_fault,,1,,,,,,hw,2,0123456789ab"
		);
	}

	#[test]
	fn test_items_and_lint_add_up_to_the_score() {
		let items = [GradingItem::new("q1"), GradingItem::new("q2")];
		let scored = |id: &str, score: f64, reason: Option<Reason>| ItemScore {
			item_id: id.into(),
			points: 1,
			aggregation: Aggregation::Proportional,
			passed: 0,
			cases: 3,
			outcome: ItemOutcome::Graded { score, reason },
		};
		let mut report = graded("0012345", 0.0);
		report.student_name = Some("张三".into());
		report.lint = Some(LintOutcome::Scored { score: 50.0 });
		if let Some(grade) = &mut report.grade {
			grade.outcome = GradeOutcome::Graded {
				score: 1.0 / 3.0 + 0.0 + 0.5,
				raw_grade: 27.78,
				final_grade: 27.78,
				reason: None,
			};
			grade.max = 3.0;
			grade.items = vec![
				scored("q1", 1.0 / 3.0, None),
				scored("q2", 0.0, Some(Reason::MissingFile)),
			];
		}
		let table = grade_table(&[report], &items, Some(1), &ORIGIN).unwrap();
		let column = |name: &str| table.header.iter().position(|h| h == name).unwrap();
		let row = &table.rows[0];
		assert_eq!(row[column("student_id")], Cell::Text("0012345".into()));
		assert_eq!(row[column("student_name")], Cell::Text("张三".into()));
		assert_eq!(row[column("q2_reason")], Cell::Text("missing_file".into()));
		let value = |name: &str| match row[column(name)] {
			Cell::Number { value, .. } => value,
			ref other => panic!("{name} is {other:?}"),
		};
		assert_eq!(value("lint"), 0.5);
		let sum = value("q1_score") + value("q2_score") + value("lint");
		// Each of the four numbers is rounded on its own.
		assert!((sum - value("score")).abs() <= 4.0 * 0.5e-4, "{sum}");
	}

	#[test]
	fn test_lint_is_shown_only_where_it_counted() {
		let mut excused = withheld("0012346", SubmissionOutcome::Executable, Reason::Excused);
		excused.excused = true;
		excused.lint = Some(LintOutcome::Scored { score: 80.0 });
		let mut linted = graded("0012347", 100.0);
		linted.lint = Some(LintOutcome::Scored { score: 80.0 });
		let table = grade_table(&[excused, linted], &[], Some(5), &ORIGIN).unwrap();
		let lint = table.header.iter().position(|h| h == "lint").unwrap();
		assert_eq!(table.rows[0][lint], Cell::Empty);
		assert_eq!(table.rows[1][lint], Cell::number(4.0, POINTS_DECIMALS));
	}

	#[test]
	fn test_cells_round_like_the_csv_and_fit_one_cell() {
		assert_eq!(
			Cell::number(-0.00001, POINTS_DECIMALS),
			Cell::Number {
				value: 0.0,
				decimals: POINTS_DECIMALS
			}
		);
		assert_eq!(Cell::number(2.0 / 3.0, POINTS_DECIMALS).csv(), "0.6667");
		let Cell::Text(long) = Cell::text("文".repeat(40_000)) else {
			panic!()
		};
		assert_eq!(long.encode_utf16().count(), CELL_LIMIT);
		assert!(long.ends_with(CUT));
		// A character outside the BMP is two UTF-16 units, and is never split.
		let Cell::Text(wide) = Cell::text("😀".repeat(20_000)) else {
			panic!()
		};
		assert!(wide.encode_utf16().count() <= CELL_LIMIT);
		assert_eq!(Cell::text("short"), Cell::Text("short".into()));
	}

	#[test]
	fn test_the_extension_picks_the_format() {
		assert_eq!(Format::of(Path::new("out/g.csv")).unwrap(), Format::Csv);
		assert_eq!(Format::of(Path::new("G.XLSX")).unwrap(), Format::Xlsx);
		for refused in ["grades", "grades.xls", "grades.json"] {
			assert!(Format::of(Path::new(refused)).is_err(), "{refused}");
		}
	}

	#[test]
	fn test_a_workbook_keeps_text_as_text_and_numbers_as_numbers() {
		use calamine::{Data, Reader, Xlsx};
		let table = Table {
			name: "grades",
			header: header(&["student_id", "student_name", "score", "actual", "empty"]),
			rows: vec![vec![
				Cell::text("0012345"),
				Cell::text("张三"),
				Cell::number(2.0 / 3.0, POINTS_DECIMALS),
				Cell::text("\u{1b}[31m=1+1\0"),
				Cell::Empty,
			]],
			keys: 2,
		};
		let bytes = workbook(&[table]).unwrap();
		let mut book = Xlsx::new(std::io::Cursor::new(bytes)).unwrap();
		let range = book.worksheet_range("grades").unwrap();
		assert_eq!(range.get((1, 0)), Some(&Data::String("0012345".into())));
		assert_eq!(range.get((1, 1)), Some(&Data::String("张三".into())));
		assert_eq!(range.get((1, 2)), Some(&Data::Float(0.6667)));
		assert_eq!(
			range.get((1, 3)),
			Some(&Data::String("\u{1b}[31m=1+1\0".into()))
		);
		assert!(matches!(range.get((1, 4)), None | Some(Data::Empty)));
	}
}
