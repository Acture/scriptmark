use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{DiagnosticKind, InputDiagnostic, SourceLocation, normalize_key};

/// One roster row. Student numbers are text — leading zeros survive, and nothing is ever
/// parsed as an integer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
	pub student_number: String,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub canvas_user_id: Option<u64>,
	#[serde(default)]
	pub location: Option<SourceLocation>,
}

impl RosterEntry {
	pub fn new(student_number: impl Into<String>, name: Option<String>) -> Self {
		Self {
			student_number: normalize_key(&student_number.into()),
			name,
			canvas_user_id: None,
			location: None,
		}
	}
}

/// The roster of record for an assignment.
///
/// Rows are kept in a `Vec`, not a map: two rows carrying the same student number are both
/// retained and reported, rather than one silently overwriting the other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
	pub entries: Vec<RosterEntry>,
	#[serde(default)]
	pub diagnostics: Vec<InputDiagnostic>,
}

/// What a key lookup found. Duplicates make "the" matching entry a question with no single
/// answer, so the caller is forced to decide rather than silently taking the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterLookup {
	Unique(usize),
	Ambiguous(Vec<usize>),
	Missing,
}

impl Roster {
	pub fn from_entries(entries: Vec<RosterEntry>) -> Self {
		let diagnostics = duplicate_diagnostics(&entries);
		Self {
			entries,
			diagnostics,
		}
	}

	/// Convenience for tests and for callers holding a plain id/name list.
	pub fn from_pairs<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> Self {
		Self::from_entries(
			pairs
				.iter()
				.map(|(id, name)| RosterEntry::new(id.as_ref(), Some(name.as_ref().to_string())))
				.collect(),
		)
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Exact-text lookup. The key is compared as written, after the same trim the loader
	/// applies — never case-folded, never zero-stripped.
	pub fn lookup(&self, key: &str) -> RosterLookup {
		let key = normalize_key(key);
		let hits: Vec<usize> = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, e)| e.student_number == key)
			.map(|(i, _)| i)
			.collect();
		match hits.len() {
			0 => RosterLookup::Missing,
			1 => RosterLookup::Unique(hits[0]),
			_ => RosterLookup::Ambiguous(hits),
		}
	}

	/// The name on a row, when there is exactly one row for that key.
	pub fn name_of(&self, key: &str) -> Option<&str> {
		match self.lookup(key) {
			RosterLookup::Unique(i) => self.entries[i].name.as_deref(),
			_ => None,
		}
	}
}

fn duplicate_diagnostics(entries: &[RosterEntry]) -> Vec<InputDiagnostic> {
	let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
	for entry in entries {
		*counts.entry(entry.student_number.as_str()).or_default() += 1;
	}
	counts
		.into_iter()
		.filter(|(_, count)| *count > 1)
		.map(|(key, count)| {
			InputDiagnostic::warning(DiagnosticKind::DuplicateRosterEntry {
				key: key.to_string(),
				count,
			})
		})
		.collect()
}

/// Load a roster CSV.
///
/// Expected format: `name,_,student_id` (header row skipped), or `name,student_id`.
/// Handles a UTF-8 BOM. Column *mapping* — choosing which column is which — is P-672;
/// this stays positional on purpose.
pub fn load_roster(path: &Path) -> Result<Roster, RosterError> {
	let content =
		std::fs::read_to_string(path).map_err(|e| RosterError::IoError(path.to_path_buf(), e))?;

	// Strip UTF-8 BOM if present
	let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

	let mut reader = csv::ReaderBuilder::new()
		.has_headers(true)
		.flexible(true)
		.from_reader(content.as_bytes());

	let mut entries = Vec::new();

	for (row, result) in reader.records().enumerate() {
		let record = result.map_err(|e| RosterError::CsvError(path.to_path_buf(), e))?;

		// Format: name, _, student_id (or name, student_id)
		let name = record.get(0).unwrap_or("").trim().to_string();
		let student_number = if record.len() >= 3 {
			record.get(2).unwrap_or("")
		} else if record.len() >= 2 {
			record.get(1).unwrap_or("")
		} else {
			continue;
		};

		let student_number = normalize_key(student_number);
		if student_number.is_empty() {
			continue;
		}

		entries.push(RosterEntry {
			student_number,
			name: (!name.is_empty()).then_some(name),
			canvas_user_id: None,
			// +2: one for the skipped header, one for 1-based line numbers.
			location: Some(SourceLocation::row(path.to_path_buf(), row + 2)),
		});
	}

	Ok(Roster::from_entries(entries))
}

#[derive(Debug, thiserror::Error)]
pub enum RosterError {
	#[error("IO error reading {0}: {1}")]
	IoError(std::path::PathBuf, std::io::Error),
	#[error("CSV parse error in {0}: {1}")]
	CsvError(std::path::PathBuf, csv::Error),
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write(dir: &tempfile::TempDir, content: &str) -> std::path::PathBuf {
		let path = dir.path().join("roster.csv");
		std::fs::write(&path, content).unwrap();
		path
	}

	#[test]
	fn test_load_roster() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nAlice,A,alice123\nBob,B,bob456\n",
		);

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.len(), 2);
		assert_eq!(roster.name_of("alice123"), Some("Alice"));
		assert_eq!(roster.name_of("bob456"), Some("Bob"));
		assert!(roster.diagnostics.is_empty());
	}

	#[test]
	fn test_load_roster_with_bom() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(&dir, "\u{feff}name,class,student_id\nAlice,A,alice123\n");

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.name_of("alice123"), Some("Alice"));
	}

	#[test]
	fn test_leading_zeros_survive_and_stay_distinct() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nCarol,C,0024010003\nDave,D,24010003\n",
		);

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.len(), 2);
		assert_eq!(roster.name_of("0024010003"), Some("Carol"));
		assert_eq!(roster.name_of("24010003"), Some("Dave"));
		// Exact-text keys cannot collide, so this is not a duplicate.
		assert!(roster.diagnostics.is_empty());
	}

	#[test]
	fn test_duplicate_rows_are_retained_and_reported() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nAlice,A,2024010001\nAlice Chen,B,2024010001\n",
		);

		let roster = load_roster(&path).unwrap();
		// Both rows survive — a HashMap would have kept one.
		assert_eq!(roster.len(), 2);
		assert_eq!(roster.diagnostics.len(), 1);
		assert!(matches!(
			&roster.diagnostics[0].kind,
			DiagnosticKind::DuplicateRosterEntry { key, count } if key == "2024010001" && *count == 2
		));

		match roster.lookup("2024010001") {
			RosterLookup::Ambiguous(hits) => assert_eq!(hits, vec![0, 1]),
			other => panic!("expected Ambiguous, got {other:?}"),
		}
		// An ambiguous key has no single name, and the loader does not invent one.
		assert_eq!(roster.name_of("2024010001"), None);
	}

	#[test]
	fn test_lookup_trims_but_does_not_otherwise_normalise() {
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		assert_eq!(roster.lookup(" 2024010001 "), RosterLookup::Unique(0));
		assert_eq!(roster.lookup("2024010001 "), RosterLookup::Unique(0));
		assert_eq!(roster.lookup("02024010001"), RosterLookup::Missing);
		assert_eq!(roster.lookup("missing"), RosterLookup::Missing);
	}

	#[test]
	fn test_rows_record_their_source_line() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(&dir, "name,class,student_id\nAlice,A,2024010001\n");

		let roster = load_roster(&path).unwrap();
		let location = roster.entries[0].location.as_ref().unwrap();
		assert_eq!(location.file.as_deref(), Some(path.as_path()));
		assert_eq!(location.row, Some(2));
	}
}
