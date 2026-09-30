use rusqlite::Row;

use crate::models::{GradeOutcome, Reason, StudentReport};

use super::{Database, DbError};

/// A grading session row.
#[derive(Debug, Clone)]
pub struct Session {
	pub id: i64,
	pub assignment: String,
	pub spec_title: Option<String>,
	pub grading_policy: Option<String>,
	pub student_count: i64,
	/// Over graded students only; `None` when nobody was graded.
	pub avg_grade: Option<f64>,
	pub created_at: String,
}

/// Whether a stored result carries a grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
	/// Saved from results that were never scored.
	Unscored,
	Graded,
	Withheld,
}

/// A result row for display.
#[derive(Debug, Clone)]
pub struct ResultRow {
	pub student_id: String,
	pub student_name: Option<String>,
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
			(RowState::Graded, Some(grade)) => format!("{grade}"),
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

const SESSION_COLUMNS: &str = "s.id, s.assignment, s.spec_title, s.grading_policy, s.student_count, s.avg_grade, s.created_at";
const RESULT_COLUMNS: &str = "r.student_id, st.name, r.pass_rate, r.grade, r.reason, r.score, \
	 r.max_score, r.raw_grade, r.final_grade, r.lint_score, r.total_cases, r.passed_cases";

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
		spec_title: row.get(at + 2)?,
		grading_policy: row.get(at + 3)?,
		student_count: row.get(at + 4)?,
		avg_grade: row.get(at + 5)?,
		created_at: row.get(at + 6)?,
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
	let state = match row.get::<_, Option<String>>(at + 3)?.as_deref() {
		None => RowState::Unscored,
		Some("graded") => RowState::Graded,
		Some("withheld") => RowState::Withheld,
		Some(other) => return Err(bad(3, format!("grade state '{other}'"))),
	};
	let reason = row
		.get::<_, Option<String>>(at + 4)?
		.map(|text| {
			serde_json::from_value::<Reason>(serde_json::Value::String(text.clone()))
				.map_err(|_| bad(4, format!("reason '{text}'")))
		})
		.transpose()?;
	Ok(ResultRow {
		student_id: row.get(at)?,
		student_name: row.get(at + 1)?,
		pass_rate: row.get(at + 2)?,
		state,
		reason,
		score: row.get(at + 5)?,
		max_score: row.get(at + 6)?,
		raw_grade: row.get(at + 7)?,
		final_grade: row.get(at + 8)?,
		lint_score: row.get(at + 9)?,
		total_cases: row.get(at + 10)?,
		passed_cases: row.get(at + 11)?,
	})
}

impl Database {
	/// Save a grading session with all student reports. Returns session ID.
	pub fn save_session(
		&self,
		assignment: &str,
		reports: &[StudentReport],
		grading_policy_json: Option<&str>,
	) -> Result<i64, DbError> {
		// Two reports for one student would be merged by UNIQUE(session_id, student_id)
		// while student_count still claimed both — the silent overwrite this model exists
		// to prevent. Refuse before writing anything.
		let mut seen = std::collections::BTreeSet::new();
		for report in reports {
			if !seen.insert(report.student_id.as_str()) {
				return Err(DbError::DuplicateStudent(report.student_id.clone()));
			}
		}

		// Average over students who actually have a grade: ungraded students would
		// otherwise drag the mean toward zero.
		let graded: Vec<f64> = reports
			.iter()
			.filter_map(StudentReport::final_grade)
			.collect();
		let avg = (!graded.is_empty()).then(|| graded.iter().sum::<f64>() / graded.len() as f64);

		self.conn.execute(
			"INSERT INTO sessions (assignment, student_count, avg_grade, grading_policy)
			 VALUES (?1, ?2, ?3, ?4)",
			rusqlite::params![assignment, reports.len() as i64, avg, grading_policy_json],
		)?;
		let session_id = self.conn.last_insert_rowid();

		let mut stmt = self.conn.prepare(
			"INSERT INTO results
			 (session_id, student_id, pass_rate, grade, reason, score, max_score, raw_grade,
			  final_grade, lint_score, total_cases, passed_cases, details)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
		)?;

		for report in reports {
			let grade = report.grade.as_ref();
			let (state, score, raw_grade) = match grade.map(|g| &g.outcome) {
				None => (None, None, None),
				Some(GradeOutcome::Graded {
					score, raw_grade, ..
				}) => (Some("graded"), Some(*score), Some(*raw_grade)),
				Some(GradeOutcome::Withheld { .. }) => (Some("withheld"), None, None),
			};
			stmt.execute(rusqlite::params![
				session_id,
				report.student_id,
				report.pass_rate(),
				state,
				grade
					.and_then(|g| g.reason())
					.map(|r| crate::export::word(&r)),
				score,
				grade.map(|g| g.max),
				raw_grade,
				report.final_grade(),
				report.lint_score(),
				report.total_cases() as i64,
				report.total_passed() as i64,
				serde_json::to_string(report)?,
			])?;
		}

		Ok(session_id)
	}

	/// List all sessions.
	pub fn list_sessions(&self) -> Result<Vec<Session>, DbError> {
		let mut stmt = self.conn.prepare(&format!(
			"SELECT {SESSION_COLUMNS} FROM sessions s ORDER BY s.created_at DESC"
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
			 ORDER BY s.created_at DESC"
		))?;
		let rows = stmt.query_map(rusqlite::params![bare, student_id], |row| {
			Ok((session_at(row, 0)?, result_at(row, 7)?))
		})?;
		Ok(rows.collect::<Result<_, _>>()?)
	}
}
