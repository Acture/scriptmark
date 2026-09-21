use super::{Database, DbError};
use crate::roster::Roster;

/// A student record from the database.
#[derive(Debug, Clone)]
pub struct Student {
	pub id: String,
	pub name: Option<String>,
	pub email: Option<String>,
	pub canvas_id: Option<i64>,
}

impl Database {
	/// Import a roster. Upserts.
	///
	/// Returns the number of rows actually stored, which is not the number of entries
	/// iterated: `students.id` is a primary key, so duplicate student numbers — which the
	/// roster deliberately keeps — collapse into one row here.
	pub fn import_roster(&self, roster: &Roster) -> Result<usize, DbError> {
		let mut stmt = self.conn.prepare(
			"INSERT INTO students (id, name, canvas_id) VALUES (?1, ?2, ?3)
			 ON CONFLICT(id) DO UPDATE SET name = excluded.name, canvas_id = excluded.canvas_id",
		)?;
		let mut stored = std::collections::BTreeSet::new();
		for entry in &roster.entries {
			stmt.execute(rusqlite::params![
				entry.student_number,
				entry.name,
				entry.canvas_user_id.map(|id| id as i64),
			])?;
			stored.insert(entry.student_number.as_str());
		}
		Ok(stored.len())
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
