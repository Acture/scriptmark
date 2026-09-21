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
			student_id: "alice".to_string(),
			student_name: Some("Alice".to_string()),
			test_results: vec![TestResult {
				spec_name: "test".to_string(),
				cases: vec![CaseResult {
					case_name: "case1".to_string(),
					status: TestStatus::Passed,
					actual: None,
					expected: None,
					failure: None,
					elapsed_ms: None,
				}],
			}],
			final_grade: Some(95.0),
			..Default::default()
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

		let report1 = vec![StudentReport {
			student_id: "alice".to_string(),
			final_grade: Some(80.0),
			..Default::default()
		}];
		let report2 = vec![StudentReport {
			student_id: "alice".to_string(),
			final_grade: Some(95.0),
			..Default::default()
		}];

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
	fn test_import_roster_counts_rows_stored_not_rows_iterated() {
		let db = Database::open_memory().unwrap();
		// The roster keeps both rows; the primary key can only hold one.
		let roster = Roster::from_pairs(&[("alice", "Alice"), ("alice", "Alice Chen")]);
		assert_eq!(roster.len(), 2);
		assert_eq!(db.import_roster(&roster).unwrap(), 1);
	}

	#[test]
	fn test_duplicate_student_ids_are_refused_rather_than_merged() {
		let db = Database::open_memory().unwrap();
		let reports = vec![
			StudentReport {
				student_id: "alice".to_string(),
				final_grade: Some(80.0),
				..Default::default()
			},
			StudentReport {
				student_id: "alice".to_string(),
				final_grade: Some(95.0),
				..Default::default()
			},
		];

		let err = db.save_session("hw5", &reports, None).unwrap_err();
		assert!(matches!(err, DbError::DuplicateStudent(id) if id == "alice"));
	}

	#[test]
	fn test_ungraded_students_read_back_as_none_not_zero() {
		let db = Database::open_memory().unwrap();
		let reports = vec![StudentReport {
			student_id: "absent".to_string(),
			submission_state: Some(SubmissionOutcome::NotSubmitted),
			..Default::default()
		}];
		let session_id = db.save_session("hw5", &reports, None).unwrap();

		// A student who was never graded must not come back as a zero.
		assert_eq!(db.get_results(session_id).unwrap()[0].final_grade, None);
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
		let reports = vec![StudentReport {
			student_id: "local:alice".to_string(),
			final_grade: Some(88.0),
			..Default::default()
		}];
		let session_id = db.save_session("hw5", &reports, None).unwrap();

		let results = db.get_results(session_id).unwrap();
		assert_eq!(results[0].student_name.as_deref(), Some("Alice Smith"));

		let history = db.get_student_history("alice").unwrap();
		assert_eq!(history.len(), 1);
		assert_eq!(history[0].1.student_name.as_deref(), Some("Alice Smith"));
	}

	#[test]
	fn test_average_ignores_ungraded_students() {
		let db = Database::open_memory().unwrap();
		let reports = vec![
			StudentReport {
				student_id: "alice".to_string(),
				final_grade: Some(90.0),
				..Default::default()
			},
			StudentReport {
				student_id: "absent".to_string(),
				submission_state: Some(SubmissionOutcome::NotSubmitted),
				..Default::default()
			},
		];

		db.save_session("hw5", &reports, None).unwrap();
		let sessions = db.list_sessions().unwrap();
		assert_eq!(sessions[0].student_count, 2);
		// A missing grade is not a zero, so it must not halve the mean.
		assert!((sessions[0].avg_grade - 90.0).abs() < 0.1);
	}
}
