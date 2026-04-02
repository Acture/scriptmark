use std::collections::HashMap;

use crate::{Database, DbError};

/// A student record from the database.
#[derive(Debug, Clone)]
pub struct Student {
	pub id: String,
	pub name: Option<String>,
	pub email: Option<String>,
	pub canvas_id: Option<i64>,
}

impl Database {
	/// Import a roster (student_id -> name mapping). Upserts.
	pub fn import_roster(&self, roster: &HashMap<String, String>) -> Result<usize, DbError> {
		let mut count = 0;
		let mut stmt = self.conn.prepare(
			"INSERT INTO students (id, name) VALUES (?1, ?2)
			 ON CONFLICT(id) DO UPDATE SET name = excluded.name",
		)?;
		for (id, name) in roster {
			stmt.execute(rusqlite::params![id, name])?;
			count += 1;
		}
		Ok(count)
	}

	/// Get a single student by ID.
	pub fn get_student(&self, id: &str) -> Result<Option<Student>, DbError> {
		let mut stmt = self
			.conn
			.prepare("SELECT id, name, email, canvas_id FROM students WHERE id = ?1")?;
		let mut rows = stmt.query_map(rusqlite::params![id], |row| {
			Ok(Student {
				id: row.get(0)?,
				name: row.get(1)?,
				email: row.get(2)?,
				canvas_id: row.get(3)?,
			})
		})?;
		match rows.next() {
			Some(Ok(s)) => Ok(Some(s)),
			Some(Err(e)) => Err(e.into()),
			None => Ok(None),
		}
	}

	/// List all students.
	pub fn list_students(&self) -> Result<Vec<Student>, DbError> {
		let mut stmt = self
			.conn
			.prepare("SELECT id, name, email, canvas_id FROM students ORDER BY id")?;
		let rows = stmt.query_map([], |row| {
			Ok(Student {
				id: row.get(0)?,
				name: row.get(1)?,
				email: row.get(2)?,
				canvas_id: row.get(3)?,
			})
		})?;
		Ok(rows.filter_map(|r| r.ok()).collect())
	}

	/// Get a student name, returning "N/A" if not found.
	pub fn get_student_name(&self, id: &str) -> String {
		self.get_student(id)
			.ok()
			.flatten()
			.and_then(|s| s.name)
			.unwrap_or_else(|| "N/A".to_string())
	}
}
