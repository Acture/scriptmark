use crate::models::StudentReport;

use super::{Database, DbError};

/// A grading session row.
#[derive(Debug, Clone)]
pub struct Session {
	pub id: i64,
	pub assignment: String,
	pub spec_title: Option<String>,
	pub grading_policy: Option<String>,
	pub student_count: i64,
	pub avg_grade: f64,
	pub created_at: String,
}

/// A result row for display.
#[derive(Debug, Clone)]
pub struct ResultRow {
	pub student_id: String,
	pub student_name: Option<String>,
	pub pass_rate: f64,
	/// `None` when the student has no grade — which is not the same as a zero.
	pub final_grade: Option<f64>,
	pub lint_score: Option<f64>,
	pub total_cases: i64,
	pub passed_cases: i64,
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
		let graded: Vec<f64> = reports.iter().filter_map(|r| r.final_grade).collect();
		let avg = if graded.is_empty() {
			0.0
		} else {
			graded.iter().sum::<f64>() / graded.len() as f64
		};

		self.conn.execute(
			"INSERT INTO sessions (assignment, student_count, avg_grade, grading_policy)
			 VALUES (?1, ?2, ?3, ?4)",
			rusqlite::params![assignment, reports.len() as i64, avg, grading_policy_json],
		)?;
		let session_id = self.conn.last_insert_rowid();

		let mut stmt = self.conn.prepare(
			"INSERT INTO results
			 (session_id, student_id, pass_rate, final_grade, lint_score, total_cases, passed_cases, details)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
		)?;

		for report in reports {
			let details = serde_json::to_string(report)?;
			stmt.execute(rusqlite::params![
				session_id,
				report.student_id,
				report.pass_rate(),
				report.final_grade,
				report.lint_score,
				report.total_cases() as i64,
				report.total_passed() as i64,
				details,
			])?;
		}

		Ok(session_id)
	}

	/// List all sessions.
	pub fn list_sessions(&self) -> Result<Vec<Session>, DbError> {
		let mut stmt = self.conn.prepare(
			"SELECT id, assignment, spec_title, grading_policy, student_count, avg_grade, created_at
			 FROM sessions ORDER BY created_at DESC",
		)?;
		let rows = stmt.query_map([], |row| {
			Ok(Session {
				id: row.get(0)?,
				assignment: row.get(1)?,
				spec_title: row.get(2)?,
				grading_policy: row.get(3)?,
				student_count: row.get(4)?,
				avg_grade: row.get(5)?,
				created_at: row.get(6)?,
			})
		})?;
		Ok(rows.filter_map(|r| r.ok()).collect())
	}

	/// Get results for a session, joined with student names.
	pub fn get_results(&self, session_id: i64) -> Result<Vec<ResultRow>, DbError> {
		let mut stmt = self.conn.prepare(
			"SELECT r.student_id, s.name, r.pass_rate, r.final_grade, r.lint_score, r.total_cases, r.passed_cases
			 FROM results r
			 -- A run made without --roster leaves keys unconfirmed, so the id carries a
			 -- `local:` prefix that students.id never does. Match either form. (substr,
			 -- not ltrim: ltrim strips a character set, so 'local:alice' would become
			 -- 'ice'.)
			 LEFT JOIN students s
			   ON s.id = CASE
			       WHEN r.student_id LIKE 'local:%' THEN substr(r.student_id, 7)
			       ELSE r.student_id
			   END
			 WHERE r.session_id = ?1
			 ORDER BY r.final_grade DESC",
		)?;
		let rows = stmt.query_map(rusqlite::params![session_id], |row| {
			Ok(ResultRow {
				student_id: row.get(0)?,
				student_name: row.get(1)?,
				pass_rate: row.get::<_, f64>(2).unwrap_or(0.0),
				final_grade: row.get(3)?,
				lint_score: row.get(4)?,
				total_cases: row.get::<_, i64>(5).unwrap_or(0),
				passed_cases: row.get::<_, i64>(6).unwrap_or(0),
			})
		})?;
		Ok(rows.filter_map(|r| r.ok()).collect())
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
		let mut stmt = self.conn.prepare(
			"SELECT s.id, s.assignment, s.spec_title, s.grading_policy, s.student_count, s.avg_grade, s.created_at,
					r.student_id, st.name, r.pass_rate, r.final_grade, r.lint_score, r.total_cases, r.passed_cases
			 FROM results r
			 JOIN sessions s ON r.session_id = s.id
			 LEFT JOIN students st
			   ON st.id = CASE
			       WHEN r.student_id LIKE 'local:%' THEN substr(r.student_id, 7)
			       ELSE r.student_id
			   END
			 WHERE r.student_id IN (?1, 'local:' || ?1, ?2)
			 ORDER BY s.created_at DESC",
		)?;
		let rows = stmt.query_map(rusqlite::params![bare, student_id], |row| {
			Ok((
				Session {
					id: row.get(0)?,
					assignment: row.get(1)?,
					spec_title: row.get(2)?,
					grading_policy: row.get(3)?,
					student_count: row.get(4)?,
					avg_grade: row.get(5)?,
					created_at: row.get(6)?,
				},
				ResultRow {
					student_id: row.get(7)?,
					student_name: row.get(8)?,
					pass_rate: row.get::<_, f64>(9).unwrap_or(0.0),
					final_grade: row.get(10)?,
					lint_score: row.get(11)?,
					total_cases: row.get::<_, i64>(12).unwrap_or(0),
					passed_cases: row.get::<_, i64>(13).unwrap_or(0),
				},
			))
		})?;
		Ok(rows.filter_map(|r| r.ok()).collect())
	}
}
