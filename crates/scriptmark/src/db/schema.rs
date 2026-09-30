use rusqlite::Connection;

use super::DbError;

/// The schema this build writes. A database at any other version — including one made
/// before grades were scored per item, which has none — is refused, not upgraded.
pub const VERSION: i64 = 1;

pub fn migrate(conn: &Connection) -> Result<(), DbError> {
	let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
	let empty: bool = conn.query_row(
		"SELECT NOT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table')",
		[],
		|row| row.get(0),
	)?;
	if version != VERSION && !(version == 0 && empty) {
		return Err(DbError::Version {
			found: version,
			expected: VERSION,
		});
	}
	conn.execute_batch(
		"
		CREATE TABLE IF NOT EXISTS students (
			id TEXT PRIMARY KEY,
			name TEXT,
			email TEXT,
			canvas_id INTEGER,
			created_at TEXT DEFAULT (datetime('now'))
		);

		CREATE TABLE IF NOT EXISTS sessions (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			assignment TEXT NOT NULL,
			spec_title TEXT,
			grading_policy TEXT,
			student_count INTEGER NOT NULL DEFAULT 0,
			-- NULL when nobody was graded.
			avg_grade REAL,
			created_at TEXT DEFAULT (datetime('now'))
		);

		CREATE TABLE IF NOT EXISTS results (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id),
			student_id TEXT NOT NULL,
			pass_rate REAL NOT NULL,
			-- 'graded' or 'withheld'; NULL when the results were never scored.
			grade TEXT CHECK (grade IN ('graded', 'withheld')),
			-- Why withheld, or why a graded 0 is a policy 0.
			reason TEXT,
			score REAL,
			max_score REAL,
			raw_grade REAL,
			-- NULL exactly when not graded: never a stand-in 0.
			final_grade REAL,
			lint_score REAL,
			total_cases INTEGER NOT NULL,
			passed_cases INTEGER NOT NULL,
			details TEXT NOT NULL,
			UNIQUE(session_id, student_id)
		);

		CREATE TABLE IF NOT EXISTS similarity (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id),
			student_a TEXT NOT NULL,
			student_b TEXT NOT NULL,
			style_score REAL,
			structure_score REAL,
			combined_score REAL
		);

		CREATE INDEX IF NOT EXISTS idx_results_session ON results(session_id);
		CREATE INDEX IF NOT EXISTS idx_results_student ON results(student_id);
		CREATE INDEX IF NOT EXISTS idx_similarity_session ON similarity(session_id);
		",
	)?;
	conn.pragma_update(None, "user_version", VERSION)?;
	Ok(())
}
