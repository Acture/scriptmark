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
		 databases from before grading records are not read — use a new file"
	)]
	Version { found: i64, expected: i64 },
	#[error("unreadable stored value: {0}")]
	Stored(String),
	#[error("'{0}' has no grade: a session stores a score revision")]
	Unscored(String),
	#[error(
		"session #{session} already holds another revision {revision} of this evidence, \
		 scored from a different copy of the record; refusing to mix them"
	)]
	Conflict { session: i64, revision: u32 },
	#[error("{0}")]
	Record(String),
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
	use scriptmark_core::models::fixtures::{graded, withheld};
	use scriptmark_core::models::*;
	use scriptmark_core::roster::Roster;
	use scriptmark_core::similarity::SimilarityPair;

	use super::*;

	/// Revision 1 of evidence named after the assignment.
	fn of(assignment: &str) -> SessionOf<'_> {
		SessionOf {
			assignment,
			evidence: assignment,
			revision: 1,
			checksum: "c1",
			bundle: "{}",
			grading_policy: "{}",
		}
	}

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

		let session_id = db.save_session(&of("hw5"), &reports).unwrap().id;
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

		db.save_session(&of("hw5"), &report1).unwrap();
		db.save_session(&of("hw8"), &report2).unwrap();

		let history = db.get_student_history("alice").unwrap();
		assert_eq!(history.len(), 2);
	}

	#[test]
	fn test_similarity_save_and_query() {
		let db = Database::open_memory().unwrap();
		let session_id = db.save_session(&of("hw5"), &[]).unwrap().id;

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

		let err = db.save_session(&of("hw5"), &reports).unwrap_err();
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
		let session_id = db.save_session(&of("hw5"), &reports).unwrap().id;

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
		let session_id = db.save_session(&of("hw5"), &reports).unwrap().id;

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
		let session_id = db.save_session(&of("hw5"), &reports).unwrap().id;

		let results = db.get_results(session_id).unwrap();
		assert_eq!(results.len(), 1, "one stored result must yield one row");
		assert_eq!(results[0].student_name.as_deref(), Some("Alice Smith"));
	}

	#[test]
	fn test_history_accepts_the_id_form_the_tables_print() {
		let db = Database::open_memory().unwrap();
		db.import_roster(&Roster::from_pairs(&[("alice", "Alice Smith")]))
			.unwrap();
		db.save_session(&of("hw5"), &[graded("local:alice", 70.0)])
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

		db.save_session(&of("hw5"), &reports).unwrap();
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
		db.save_session(&of("hw5"), &reports).unwrap();
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
		];
		let session_id = db.save_session(&of("hw5"), &reports).unwrap().id;
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
		// Graded rows come first, withheld after.
		assert!(rows[..2].iter().all(|r| r.state == RowState::Graded));
	}

	#[test]
	fn test_a_database_reopens_but_an_old_one_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("grades.db");
		Database::open(&path)
			.unwrap()
			.save_session(&of("hw5"), &[])
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
			.save_session(&of("hw5"), &[graded("alice", 90.0)])
			.unwrap()
			.id;
		db.conn
			.execute("UPDATE results SET reason = 'no_such_reason'", [])
			.unwrap();
		assert!(db.get_results(session_id).is_err());
	}

	#[test]
	fn test_a_revision_is_saved_once_and_found_again() {
		let db = Database::open_memory().unwrap();
		let reports = [graded("alice", 90.0)];
		let first = db.save_session(&of("hw5"), &reports).unwrap();
		let again = db.save_session(&of("hw5"), &reports).unwrap();
		assert!(first.created);
		assert_eq!(
			again,
			Saved {
				id: first.id,
				created: false
			}
		);
		let second = SessionOf {
			revision: 2,
			..of("hw5")
		};
		assert!(db.save_session(&second, &reports).unwrap().created);
		let sessions = db.list_sessions().unwrap();
		assert_eq!(
			sessions.iter().map(|s| s.revision).collect::<Vec<_>>(),
			[2, 1],
			"newest first, even within one second"
		);
		assert!(sessions.iter().all(|s| s.evidence == "hw5"));
	}

	#[test]
	fn test_an_unscored_report_is_not_a_session() {
		let db = Database::open_memory().unwrap();
		let reports = [StudentReport::new("dan", SubmissionOutcome::Executable)];
		let err = db.save_session(&of("hw5"), &reports).unwrap_err();
		assert!(matches!(err, DbError::Unscored(id) if id == "dan"));
		assert!(
			db.list_sessions().unwrap().is_empty(),
			"nothing half-written"
		);
	}

	#[test]
	fn test_identity_fault_and_evidence_version_read_back() {
		let db = Database::open_memory().unwrap();
		// The roster says otherwise; the record's own name wins.
		db.import_roster(&Roster::from_pairs(&[("alice", "Someone Else")]))
			.unwrap();
		let report = StudentReport {
			student_name: Some("Alice Wu".into()),
			canvas_user_id: Some(4242),
			test_results: vec![TestResult {
				item_id: "q".into(),
				file: Some("/subs/alice_q.py".into()),
				cases: vec![CaseResult {
					case_name: "one".into(),
					status: TestStatus::Error,
					fault: Some(Fault::Teacher),
					cause: Some(Cause::TeacherImport),
					..Default::default()
				}],
			}],
			submission: Some(SubmissionVersion {
				attempt: Some(2),
				submitted_at: Some("2026-10-01T08:00:00Z".into()),
				files: vec![FileVersion {
					path: "/subs/alice_q.py".into(),
					sha256: "ab".into(),
				}],
				archives: Vec::new(),
			}),
			..withheld("alice", SubmissionOutcome::Executable, Reason::TeacherFault)
		};
		let session = db
			.save_session(
				&SessionOf {
					revision: 3,
					..of("hw5")
				},
				std::slice::from_ref(&report),
			)
			.unwrap()
			.id;

		let row = &db.get_results(session).unwrap()[0];
		assert_eq!(row.student_name.as_deref(), Some("Alice Wu"));
		assert_eq!(row.canvas_user_id, Some(4242));
		let stored = db.get_student_details(session, "alice").unwrap().unwrap();
		let case = &stored.test_results[0].cases[0];
		assert_eq!(
			(case.fault, case.cause),
			(Some(Fault::Teacher), Some(Cause::TeacherImport))
		);
		assert_eq!(stored.submission, report.submission);
		assert_eq!(stored.grade, report.grade);
		let (session, _) = &db.get_student_history("alice").unwrap()[0];
		assert_eq!((session.evidence.as_str(), session.revision), ("hw5", 3));
	}

	#[test]
	fn test_a_database_from_before_grading_records_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let old = dir.path().join("v1.db");
		rusqlite::Connection::open(&old)
			.unwrap()
			.execute_batch(
				"CREATE TABLE sessions (id INTEGER PRIMARY KEY); PRAGMA user_version = 1;",
			)
			.unwrap();
		let err = Database::open(&old)
			.err()
			.expect("a version 1 database is refused");
		assert!(
			matches!(
				err,
				DbError::Version {
					found: 1,
					expected: 2
				}
			),
			"{err}"
		);
	}

	#[test]
	fn test_another_revision_under_a_saved_number_is_refused() {
		let db = Database::open_memory().unwrap();
		let reports = [graded("alice", 90.0)];
		let id = db.save_session(&of("hw5"), &reports).unwrap().id;
		let other = SessionOf {
			checksum: "c2",
			..of("hw5")
		};
		let err = db.save_session(&other, &reports).unwrap_err();
		assert!(
			matches!(err, DbError::Conflict { session, revision: 1 } if session == id),
			"{err}"
		);
	}

	#[test]
	fn test_a_session_that_fails_part_way_leaves_nothing() {
		let db = Database::open_memory().unwrap();
		db.conn
			.execute_batch(
				"CREATE TRIGGER fail BEFORE INSERT ON results WHEN NEW.student_id = 'bob'
				 BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
			)
			.unwrap();
		let reports = [graded("alice", 90.0), graded("bob", 80.0)];
		assert!(db.save_session(&of("hw5"), &reports).is_err());
		assert!(db.list_sessions().unwrap().is_empty(), "no half session");
		db.conn.execute_batch("DROP TRIGGER fail").unwrap();
		assert!(db.save_session(&of("hw5"), &reports).unwrap().created);
	}
}
