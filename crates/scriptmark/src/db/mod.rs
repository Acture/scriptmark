mod queries;
mod results;
mod roster;
mod schema;

pub use results::*;
pub use roster::*;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
	#[error("SQLite error: {0}")]
	Sqlite(#[from] rusqlite::Error),
	#[error("JSON serialization error: {0}")]
	Json(#[from] serde_json::Error),
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
	#[error("two reports share student id '{0}'; refusing to overwrite one with the other")]
	DuplicateStudent(String),
	#[error(
		"this database is schema version {found}, and this build reads only {expected}; \
		 databases from before per-item grading are not read — use a new file"
	)]
	Version { found: i64, expected: i64 },
	#[error("unreadable stored value: {0}")]
	Stored(String),
}

pub struct Database {
	conn: Connection,
}

impl Database {
	/// Open or create a database at the given path.
	pub fn open(path: &Path) -> Result<Self, DbError> {
		let conn = Connection::open(path)?;
		conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
		let db = Self { conn };
		db.migrate()?;
		Ok(db)
	}

	/// Open an in-memory database (for testing).
	pub fn open_memory() -> Result<Self, DbError> {
		let conn = Connection::open_in_memory()?;
		conn.execute_batch("PRAGMA foreign_keys=ON;")?;
		let db = Self { conn };
		db.migrate()?;
		Ok(db)
	}

	fn migrate(&self) -> Result<(), DbError> {
		schema::migrate(&self.conn)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use crate::models::fixtures::{graded, withheld};
	use crate::models::*;
	use crate::roster::Roster;
	use crate::similarity::SimilarityPair;

	use super::*;

	#[test]
	fn test_open_memory() {
		let db = Database::open_memory().unwrap();
		assert!(db.list_students().unwrap().is_empty());
	}

	#[test]
	fn test_roster_import_and_query() {
		let db = Database::open_memory().unwrap();
		let roster = Roster::from_pairs(&[("alice", "Alice Smith"), ("bob", "Bob Jones")]);

		let count = db.import_roster(&roster).unwrap();
		assert_eq!(count, 2);

		let students = db.list_students().unwrap();
		assert_eq!(students.len(), 2);

		let alice = db.get_student("alice").unwrap().unwrap();
		assert_eq!(alice.name.as_deref(), Some("Alice Smith"));
	}

	#[test]
	fn test_save_and_query_session() {
		let db = Database::open_memory().unwrap();

		let reports = vec![StudentReport {
			student_name: Some("Alice".to_string()),
			test_results: vec![TestResult {
				file: None,
				item_id: "test".to_string(),
				cases: vec![CaseResult {
					case_name: "case1".to_string(),
					status: TestStatus::Passed,
					actual: None,
					expected: None,
					failure: None,
					elapsed_ms: None,
					..Default::default()
				}],
			}],
			..graded("alice", 95.0)
		}];

		let session_id = db.save_session("hw5", &reports, None).unwrap();
		assert!(session_id > 0);

		let sessions = db.list_sessions().unwrap();
		assert_eq!(sessions.len(), 1);
		assert_eq!(sessions[0].assignment, "hw5");
		assert_eq!(sessions[0].student_count, 1);

		let results = db.get_results(session_id).unwrap();
		assert_eq!(results.len(), 1);
		assert_eq!(results[0].student_id, "alice");
		assert_eq!(results[0].final_grade, Some(95.0));
	}

	#[test]
	fn test_student_history() {
		let db = Database::open_memory().unwrap();

		let report1 = vec![graded("alice", 80.0)];
		let report2 = vec![graded("alice", 95.0)];

		db.save_session("hw5", &report1, None).unwrap();
		db.save_session("hw8", &report2, None).unwrap();

		let history = db.get_student_history("alice").unwrap();
		assert_eq!(history.len(), 2);
	}

	#[test]
	fn test_similarity_save_and_query() {
		let db = Database::open_memory().unwrap();
		let session_id = db.save_session("hw5", &[], None).unwrap();

		let pairs = vec![SimilarityPair {
			student_a: "alice".to_string(),
			student_b: "bob".to_string(),
			style_score: 0.95,
			structure_score: 0.88,
			score: 0.95,
		}];

		db.save_similarity(session_id, &pairs).unwrap();

		let loaded = db.get_similarity(session_id).unwrap();
		assert_eq!(loaded.len(), 1);
		assert_eq!(loaded[0].student_a, "alice");
		assert!((loaded[0].score - 0.95).abs() < 0.01);
	}

	#[test]
	fn test_roster_upsert() {
		let db = Database::open_memory().unwrap();
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice V1")]))
			.unwrap();
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice V2")]))
			.unwrap();

		let alice = db.get_student("alice").unwrap().unwrap();
		assert_eq!(alice.name.as_deref(), Some("Alice V2"));

		assert_eq!(db.list_students().unwrap().len(), 1);
	}

	#[test]
	fn test_import_roster_counts_rows_stored() {
		let db = Database::open_memory().unwrap();
		// A repeated row is merged before it ever reaches the database.
		let roster = Roster::from_pairs(&[("alice", "Alice"), ("alice", "Alice")]);
		assert_eq!(roster.len(), 1);
		assert_eq!(db.import_roster(&roster).unwrap(), 1);
	}

	#[test]
	fn test_duplicate_student_ids_are_refused_rather_than_merged() {
		let db = Database::open_memory().unwrap();
		let reports = vec![graded("alice", 80.0), graded("alice", 95.0)];

		let err = db.save_session("hw5", &reports, None).unwrap_err();
		assert!(matches!(err, DbError::DuplicateStudent(id) if id == "alice"));
	}

	#[test]
	fn test_ungraded_students_read_back_as_none_not_zero() {
		let db = Database::open_memory().unwrap();
		let reports = vec![withheld(
			"absent",
			SubmissionOutcome::NotSubmitted,
			Reason::NotSubmitted,
		)];
		let session_id = db.save_session("hw5", &reports, None).unwrap();

		// A student who was never graded must not come back as a zero.
		let row = &db.get_results(session_id).unwrap()[0];
		assert_eq!(row.final_grade, None);
		assert_eq!(row.state, RowState::Withheld);
		assert_eq!(row.reason, Some(Reason::NotSubmitted));
		assert_eq!(
			db.get_student_history("absent").unwrap()[0].1.final_grade,
			None
		);
	}

	#[test]
	fn test_an_unconfirmed_key_still_joins_to_its_roster_row() {
		let db = Database::open_memory().unwrap();
		// A run made without --roster renders ids with a `local:` prefix; the roster
		// imported afterwards holds the bare token. Both must resolve to one student —
		// including a non-numeric one, which a character-set trim would have mangled.
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice Smith")]))
			.unwrap();
		let reports = vec![graded("local:alice", 88.0)];
		let session_id = db.save_session("hw5", &reports, None).unwrap();

		let results = db.get_results(session_id).unwrap();
		assert_eq!(results[0].student_name.as_deref(), Some("Alice Smith"));

		let history = db.get_student_history("alice").unwrap();
		assert_eq!(history.len(), 1);
		assert_eq!(history[0].1.student_name.as_deref(), Some("Alice Smith"));
	}

	#[test]
	fn test_a_result_row_never_picks_up_a_second_students_name() {
		let db = Database::open_memory().unwrap();
		// Both forms present in the students table: the join must resolve to exactly one.
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice Smith")]))
			.unwrap();
		db.conn
			.execute(
				"INSERT INTO students (id, name) VALUES ('local:alice', 'Someone Else')",
				[],
			)
			.unwrap();

		let reports = vec![graded("local:alice", 88.0)];
		let session_id = db.save_session("hw5", &reports, None).unwrap();

		let results = db.get_results(session_id).unwrap();
		assert_eq!(results.len(), 1, "one stored result must yield one row");
		assert_eq!(results[0].student_name.as_deref(), Some("Alice Smith"));
	}

	#[test]
	fn test_history_accepts_the_id_form_the_tables_print() {
		let db = Database::open_memory().unwrap();
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice Smith")]))
			.unwrap();
		db.save_session("hw5", &[graded("local:alice", 70.0)], None)
			.unwrap();

		// Whichever form the teacher copies out of the summary must find the run.
		for id in ["alice", "local:alice"] {
			let history = db.get_student_history(id).unwrap();
			assert_eq!(history.len(), 1, "no history for '{id}'");
			assert_eq!(history[0].1.student_name.as_deref(), Some("Alice Smith"));
			assert_eq!(db.get_student_name(id), "Alice Smith");
		}
	}

	#[test]
	fn test_a_csv_import_does_not_erase_a_stored_canvas_id() {
		let db = Database::open_memory().unwrap();
		// First a Canvas-sourced import, which knows the Canvas id...
		let mut from_canvas = Roster::from_pairs(&[("alice", "Alice")]);
		from_canvas.entries[0].canvas_user_id = Some(4242);
		db.import_roster(&from_canvas).unwrap();

		// ...then a CSV, which never carries one. It must not null the id out, or grade
		// push loses the only thing it can key on.
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice Wu")]))
			.unwrap();

		let stored = db.get_student("alice").unwrap().unwrap();
		assert_eq!(stored.name.as_deref(), Some("Alice Wu"));
		assert_eq!(stored.canvas_id, Some(4242));
	}

	#[test]
	fn test_average_ignores_ungraded_students() {
		let db = Database::open_memory().unwrap();
		let reports = vec![
			graded("alice", 90.0),
			withheld(
				"absent",
				SubmissionOutcome::NotSubmitted,
				Reason::NotSubmitted,
			),
		];

		db.save_session("hw5", &reports, None).unwrap();
		let sessions = db.list_sessions().unwrap();
		assert_eq!(sessions[0].student_count, 2);
		// A missing grade is not a zero, so it must not halve the mean.
		assert_eq!(sessions[0].avg_grade, Some(90.0));
	}

	#[test]
	fn test_session_average_is_none_when_nobody_is_graded() {
		let db = Database::open_memory().unwrap();
		let reports = [withheld(
			"alice",
			SubmissionOutcome::Executable,
			Reason::TeacherFault,
		)];
		db.save_session("hw5", &reports, None).unwrap();
		assert_eq!(db.list_sessions().unwrap()[0].avg_grade, None);
	}

	#[test]
	fn test_a_zero_and_a_withheld_grade_read_back_apart() {
		let db = Database::open_memory().unwrap();
		let mut policy_zero = graded("bob", 0.0);
		if let Some(Grade {
			outcome: GradeOutcome::Graded { reason, .. },
			..
		}) = &mut policy_zero.grade
		{
			*reason = Some(Reason::NotSubmitted);
		}
		let reports = [
			graded("alice", 0.0),
			policy_zero,
			withheld(
				"carol",
				SubmissionOutcome::Executable,
				Reason::EnvironmentFault,
			),
			StudentReport::new("dan", SubmissionOutcome::Executable),
		];
		let session_id = db.save_session("hw5", &reports, None).unwrap();
		let rows = db.get_results(session_id).unwrap();
		let row = |id: &str| rows.iter().find(|r| r.student_id == id).unwrap();

		assert_eq!(row("alice").state, RowState::Graded);
		assert_eq!(row("alice").final_grade, Some(0.0));
		assert_eq!(row("alice").reason, None);
		assert_eq!(row("bob").final_grade, Some(0.0));
		assert_eq!(row("bob").reason, Some(Reason::NotSubmitted));
		assert_eq!(row("carol").state, RowState::Withheld);
		assert_eq!(row("carol").final_grade, None);
		assert_eq!(row("carol").reason, Some(Reason::EnvironmentFault));
		assert_eq!(row("dan").state, RowState::Unscored);
		// Graded rows come first, withheld and unscored after.
		assert!(rows[..2].iter().all(|r| r.state == RowState::Graded));
	}

	#[test]
	fn test_a_database_reopens_but_an_old_one_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("grades.db");
		Database::open(&path)
			.unwrap()
			.save_session("hw5", &[], None)
			.unwrap();
		let reopened = Database::open(&path).unwrap();
		assert_eq!(reopened.list_sessions().unwrap().len(), 1);

		// A database from before per-item grading: tables, but no schema version.
		let old = dir.path().join("old.db");
		rusqlite::Connection::open(&old)
			.unwrap()
			.execute_batch(
				"CREATE TABLE sessions (id INTEGER PRIMARY KEY, avg_grade REAL DEFAULT 0);",
			)
			.unwrap();
		let err = Database::open(&old)
			.err()
			.expect("an old database is refused");
		assert!(matches!(err, DbError::Version { found: 0, .. }), "{err}");
	}

	#[test]
	fn test_an_unknown_stored_state_is_an_error_not_a_dropped_row() {
		let db = Database::open_memory().unwrap();
		let session_id = db
			.save_session("hw5", &[graded("alice", 90.0)], None)
			.unwrap();
		db.conn
			.execute("UPDATE results SET reason = 'no_such_reason'", [])
			.unwrap();
		assert!(db.get_results(session_id).is_err());
	}
}
