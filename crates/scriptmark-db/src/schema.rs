use rusqlite::Connection;

use crate::DbError;

pub fn migrate(conn: &Connection) -> Result<(), DbError> {
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
			student_count INTEGER DEFAULT 0,
			avg_grade REAL DEFAULT 0,
			created_at TEXT DEFAULT (datetime('now'))
		);

		CREATE TABLE IF NOT EXISTS results (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id),
			student_id TEXT NOT NULL,
			pass_rate REAL,
			final_grade REAL,
			lint_score REAL,
			total_cases INTEGER,
			passed_cases INTEGER,
			details TEXT,
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
    Ok(())
}
