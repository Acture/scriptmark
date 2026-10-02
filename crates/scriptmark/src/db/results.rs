use rusqlite::{OptionalExtension, Row};

use crate::models::{GradeOutcome, Reason, StudentReport};
use crate::record::Record;

use super::{Database, DbError};

/// A grading session: one score revision of one grading record's evidence.
#[derive(Debug, Clone)]
pub struct Session {
	pub id: i64,
	pub assignment: String,
	/// The digest of the evidence the revision scored.
	pub evidence: String,
	pub revision: u32,
	/// The test bundle's version, as the record holds it (JSON).
	pub bundle: String,
	/// The revision's items and grading policy (JSON).
	pub grading_policy: String,
	pub student_count: i64,
	/// Over graded students only; `None` when nobody was graded.
	pub avg_grade: Option<f64>,
	pub created_at: String,
}

/// What a session is a session of.
#[derive(Debug, Clone, Copy)]
pub struct SessionOf<'a> {
	pub assignment: &'a str,
	pub evidence: &'a str,
	pub revision: u32,
	pub bundle: &'a str,
	pub grading_policy: &'a str,
}

/// A stored session, and whether this save stored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
	pub id: i64,
	/// `false` when the revision was already saved, as session `id`.
	pub created: bool,
}

/// Whether a stored result is a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
	Graded,
	Withheld,
}

/// A result row for display.
#[derive(Debug, Clone)]
pub struct ResultRow {
	pub student_id: String,
	pub student_name: Option<String>,
	pub canvas_user_id: Option<u64>,
	pub pass_rate: f64,
	pub state: RowState,
	/// Why withheld, or why a graded 0 is a policy 0.
	pub reason: Option<Reason>,
	pub score: Option<f64>,
	pub max_score: Option<f64>,
	pub raw_grade: Option<f64>,
	/// `None` when the student has no grade — which is not the same as a zero.
	pub final_grade: Option<f64>,
	pub lint_score: Option<f64>,
	pub total_cases: i64,
	pub passed_cases: i64,
}

impl ResultRow {
	/// Share of the points earned, for colouring: `None` without a grade.
	pub fn fraction(&self) -> Option<f64> {
		match (self.state, self.score, self.max_score) {
			(RowState::Graded, Some(score), Some(max)) if max > 0.0 => Some(score / max),
			_ => None,
		}
	}

	/// The grade as a cell: the number, or why there is none — never a stand-in 0.
	pub fn grade_text(&self) -> String {
		match (self.state, self.final_grade) {
			(RowState::Graded, Some(grade)) => {
				crate::export::number(grade, crate::export::POINTS_DECIMALS)
			}
			(RowState::Withheld, _) => format!(
				"- ({})",
				self.reason
					.map(|r| crate::export::word(&r))
					.unwrap_or_default()
			),
			_ => "-".into(),
		}
	}
}

const SESSION_COLUMNS: &str = "s.id, s.assignment, s.evidence, s.revision, s.bundle, \
	 s.grading_policy, s.student_count, s.avg_grade, s.created_at";
/// The name the record stored, else the roster's.
const RESULT_COLUMNS: &str = "r.student_id, COALESCE(r.student_name, st.name), r.canvas_user_id, \
	 r.pass_rate, r.grade, r.reason, r.score, r.max_score, r.raw_grade, r.final_grade, \
	 r.lint_score, r.total_cases, r.passed_cases";
/// Newest first; a revision saved in the same second as its predecessor still sorts after it.
const NEWEST_FIRST: &str = "s.created_at DESC, s.id DESC";

/// A run made without --roster leaves keys unconfirmed, so the id carries a `local:` prefix
/// that students.id never does. Match either form. (substr, not ltrim: ltrim strips a
/// character set, so 'local:alice' would become 'ice'.)
const JOIN_STUDENT: &str = "LEFT JOIN students st
	ON st.id = CASE
		WHEN r.student_id LIKE 'local:%' THEN substr(r.student_id, 7)
		ELSE r.student_id
	END";

fn session_at(row: &Row, at: usize) -> rusqlite::Result<Session> {
	Ok(Session {
		id: row.get(at)?,
		assignment: row.get(at + 1)?,
		evidence: row.get(at + 2)?,
		revision: row.get(at + 3)?,
		bundle: row.get(at + 4)?,
		grading_policy: row.get(at + 5)?,
		student_count: row.get(at + 6)?,
		avg_grade: row.get(at + 7)?,
		created_at: row.get(at + 8)?,
	})
}

/// Read a result row strictly: an unknown state or reason is an error, never a guess.
fn result_at(row: &Row, at: usize) -> rusqlite::Result<ResultRow> {
	let bad = |i: usize, what: String| {
		rusqlite::Error::FromSqlConversionFailure(
			at + i,
			rusqlite::types::Type::Text,
			Box::new(DbError::Stored(what)),
		)
	};
	let state = match row.get::<_, String>(at + 4)?.as_str() {
		"graded" => RowState::Graded,
		"withheld" => RowState::Withheld,
		other => return Err(bad(4, format!("grade state '{other}'"))),
	};
	let reason = row
		.get::<_, Option<String>>(at + 5)?
		.map(|text| {
			serde_json::from_value::<Reason>(serde_json::Value::String(text.clone()))
				.map_err(|_| bad(5, format!("reason '{text}'")))
		})
		.transpose()?;
	let canvas_user_id = row
		.get::<_, Option<i64>>(at + 2)?
		.map(|id| u64::try_from(id).map_err(|_| bad(2, format!("Canvas user id {id}"))))
		.transpose()?;
	Ok(ResultRow {
		student_id: row.get(at)?,
		student_name: row.get(at + 1)?,
		canvas_user_id,
		pass_rate: row.get(at + 3)?,
		state,
		reason,
		score: row.get(at + 6)?,
		max_score: row.get(at + 7)?,
		raw_grade: row.get(at + 8)?,
		final_grade: row.get(at + 9)?,
		lint_score: row.get(at + 10)?,
		total_cases: row.get(at + 11)?,
		passed_cases: row.get(at + 12)?,
	})
}

impl Database {
	/// Save revision `revision` of `record` as a session. A revision already saved is found,
	/// not saved twice.
	pub fn save_revision(&self, record: &Record, revision: u32) -> Result<Saved, DbError> {
		let invalid = |e: anyhow::Error| DbError::Record(format!("{e:#}"));
		let policy = &record.revision(revision).map_err(invalid)?.policy;
		let view = record.view(Some(revision)).map_err(invalid)?;
		self.save_session(
			&SessionOf {
				assignment: &record.evidence.assignment.name,
				evidence: &record.digest,
				revision,
				bundle: &serde_json::to_string(&record.evidence.bundle)?,
				grading_policy: &serde_json::to_string(policy)?,
			},
			&view.reports,
		)
	}

	/// Save scored reports as a session, in one transaction. Returns the session it already
	/// has when this revision of this evidence was saved before.
	pub fn save_session(
		&self,
		of: &SessionOf,
		reports: &[StudentReport],
	) -> Result<Saved, DbError> {
		// Two reports for one student would be merged by UNIQUE(session_id, student_id)
		// while student_count still claimed both — the silent overwrite this model exists
		// to prevent. Refuse before writing anything.
		let mut seen = std::collections::BTreeSet::new();
		for report in reports {
			if !seen.insert(report.student_id.as_str()) {
				return Err(DbError::DuplicateStudent(report.student_id.clone()));
			}
		}
		if let Some(report) = reports.iter().find(|r| r.grade.is_none()) {
			return Err(DbError::Unscored(report.student_id.clone()));
		}
		if let Some(id) = self
			.conn
			.query_row(
				"SELECT id FROM sessions WHERE evidence = ?1 AND revision = ?2",
				rusqlite::params![of.evidence, of.revision],
				|row| row.get(0),
			)
			.optional()?
		{
			return Ok(Saved { id, created: false });
		}

		// Average over students who actually have a grade: ungraded students would
		// otherwise drag the mean toward zero.
		let graded: Vec<f64> = reports
			.iter()
			.filter_map(StudentReport::final_grade)
			.collect();
		let avg = (!graded.is_empty()).then(|| graded.iter().sum::<f64>() / graded.len() as f64);

		let tx = self.conn.unchecked_transaction()?;
		tx.execute(
			"INSERT INTO sessions
			 (assignment, evidence, revision, bundle, grading_policy, student_count, avg_grade)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
			rusqlite::params![
				of.assignment,
				of.evidence,
				of.revision,
				of.bundle,
				of.grading_policy,
				reports.len() as i64,
				avg
			],
		)?;
		let session_id = tx.last_insert_rowid();
		{
			let mut stmt = tx.prepare(
				"INSERT INTO results
				 (session_id, student_id, student_name, canvas_user_id, pass_rate, grade, reason,
				  score, max_score, raw_grade, final_grade, lint_score, total_cases,
				  passed_cases, details)
				 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
			)?;
			for report in reports {
				let grade = report.grade.as_ref().expect("checked above");
				let (state, score, raw_grade) = match &grade.outcome {
					GradeOutcome::Graded {
						score, raw_grade, ..
					} => ("graded", Some(*score), Some(*raw_grade)),
					GradeOutcome::Withheld { .. } => ("withheld", None, None),
				};
				stmt.execute(rusqlite::params![
					session_id,
					report.student_id,
					report.student_name,
					report.canvas_user_id.map(|id| id as i64),
					report.pass_rate(),
					state,
					grade.reason().map(|r| crate::export::word(&r)),
					score,
					grade.max,
					raw_grade,
					report.final_grade(),
					report.lint_score(),
					report.total_cases() as i64,
					report.total_passed() as i64,
					serde_json::to_string(report)?,
				])?;
			}
		}
		tx.commit()?;
		Ok(Saved {
			id: session_id,
			created: true,
		})
	}

	/// List all sessions.
	pub fn list_sessions(&self) -> Result<Vec<Session>, DbError> {
		let mut stmt = self.conn.prepare(&format!(
			"SELECT {SESSION_COLUMNS} FROM sessions s ORDER BY {NEWEST_FIRST}"
		))?;
		let rows = stmt.query_map([], |row| session_at(row, 0))?;
		Ok(rows.collect::<Result<_, _>>()?)
	}

	/// Get results for a session, joined with student names; graded first, best first.
	pub fn get_results(&self, session_id: i64) -> Result<Vec<ResultRow>, DbError> {
		let mut stmt = self.conn.prepare(&format!(
			"SELECT {RESULT_COLUMNS}
			 FROM results r
			 {JOIN_STUDENT}
			 WHERE r.session_id = ?1
			 ORDER BY r.final_grade DESC NULLS LAST, r.student_id"
		))?;
		let rows = stmt.query_map(rusqlite::params![session_id], |row| result_at(row, 0))?;
		Ok(rows.collect::<Result<_, _>>()?)
	}

	/// Get full StudentReport JSON for a specific student in a session.
	pub fn get_student_details(
		&self,
		session_id: i64,
		student_id: &str,
	) -> Result<Option<StudentReport>, DbError> {
		let mut stmt = self
			.conn
			.prepare("SELECT details FROM results WHERE session_id = ?1 AND student_id = ?2")?;
		let mut rows = stmt.query_map(rusqlite::params![session_id, student_id], |row| {
			let json: String = row.get(0)?;
			Ok(json)
		})?;
		match rows.next() {
			Some(Ok(json)) => {
				let report: StudentReport = serde_json::from_str(&json)?;
				Ok(Some(report))
			}
			Some(Err(e)) => Err(e.into()),
			None => Ok(None),
		}
	}

	/// Get a student's history across all sessions.
	/// Accepts the id in whichever form the teacher read off a table: a bare 学号, or the
	/// `local:`-prefixed form a run made without a roster prints.
	pub fn get_student_history(
		&self,
		student_id: &str,
	) -> Result<Vec<(Session, ResultRow)>, DbError> {
		let bare = student_id
			.strip_prefix("local:")
			.unwrap_or(student_id)
			.to_string();
		let mut stmt = self.conn.prepare(&format!(
			"SELECT {SESSION_COLUMNS}, {RESULT_COLUMNS}
			 FROM results r
			 JOIN sessions s ON r.session_id = s.id
			 {JOIN_STUDENT}
			 WHERE r.student_id IN (?1, 'local:' || ?1, ?2)
			 ORDER BY {NEWEST_FIRST}"
		))?;
		let rows = stmt.query_map(rusqlite::params![bare, student_id], |row| {
			Ok((session_at(row, 0)?, result_at(row, 9)?))
		})?;
		Ok(rows.collect::<Result<_, _>>()?)
	}
}
