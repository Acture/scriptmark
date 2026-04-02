use scriptmark_core::similarity::SimilarityPair;

use crate::{Database, DbError};

impl Database {
	/// Save similarity pairs for a session.
	pub fn save_similarity(
		&self,
		session_id: i64,
		pairs: &[SimilarityPair],
	) -> Result<(), DbError> {
		let mut stmt = self.conn.prepare(
			"INSERT INTO similarity (session_id, student_a, student_b, style_score, structure_score, combined_score)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
		)?;
		for pair in pairs {
			stmt.execute(rusqlite::params![
				session_id,
				pair.student_a,
				pair.student_b,
				pair.style_score,
				pair.structure_score,
				pair.score,
			])?;
		}
		Ok(())
	}

	/// Get similarity pairs for a session.
	pub fn get_similarity(&self, session_id: i64) -> Result<Vec<SimilarityPair>, DbError> {
		let mut stmt = self.conn.prepare(
			"SELECT student_a, student_b, style_score, structure_score, combined_score
			 FROM similarity WHERE session_id = ?1 ORDER BY combined_score DESC",
		)?;
		let rows = stmt.query_map(rusqlite::params![session_id], |row| {
			Ok(SimilarityPair {
				student_a: row.get(0)?,
				student_b: row.get(1)?,
				style_score: row.get(2)?,
				structure_score: row.get(3)?,
				score: row.get(4)?,
			})
		})?;
		Ok(rows.filter_map(|r| r.ok()).collect())
	}
}
