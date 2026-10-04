use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{
	DiagnosticKind, DiagnosticSeverity, InputDiagnostic, SourceLocation, StudentKey, normalize_key,
};

/// One roster row.
///
/// The key is a [`StudentKey`], not a bare string, so a Canvas enrollment carrying no SIS
/// id is still a roster member — keyed by its Canvas id — rather than being dropped for
/// want of a student number. Student numbers are text: leading zeros survive, and nothing
/// is ever parsed as an integer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterSource {
	/// A row the teacher supplied, from a CSV or an explicit config.
	#[default]
	Supplied,
	/// A course enrollment Canvas reported.
	CanvasEnrollment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
	pub key: StudentKey,
	#[serde(default)]
	pub source: RosterSource,
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
			key: StudentKey::Number(normalize_key(&student_number.into())),
			source: RosterSource::Supplied,
			name,
			canvas_user_id: None,
			location: None,
		}
	}

	/// The 学号, when this row has one.
	pub fn student_number(&self) -> Option<&str> {
		match &self.key {
			StudentKey::Number(number) => Some(number),
			_ => None,
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

impl Roster {
	pub fn from_entries(entries: Vec<RosterEntry>) -> Self {
		Self::with_diagnostics(entries, Vec::new())
	}

	pub fn with_diagnostics(
		entries: Vec<RosterEntry>,
		mut diagnostics: Vec<InputDiagnostic>,
	) -> Self {
		let (entries, merge_diagnostics) = merge_by_key(entries);
		diagnostics.extend(merge_diagnostics);
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

	/// Exact-key lookup, returning the one row for that key.
	///
	/// A student number is compared as written, after the same trim the loader applies —
	/// never case-folded, never zero-stripped. An unconfirmed local token and a confirmed
	/// 学号 denote the same thing, so they match the same row; a Canvas id is a separate
	/// namespace and only ever matches a Canvas-keyed row.
	///
	/// There is at most one row per key: a student number identifies one person, so rows
	/// that merely repeat are merged on construction and rows that contradict each other
	/// are refused.
	pub fn lookup(&self, key: &StudentKey) -> Option<usize> {
		let matches = |entry: &RosterEntry| match (&entry.key, key) {
			(StudentKey::CanvasUser(a), StudentKey::CanvasUser(b)) => a == b,
			(StudentKey::CanvasUser(_), _) | (_, StudentKey::CanvasUser(_)) => false,
			(a, b) => a.raw() == b.raw(),
		};
		self.entries.iter().position(matches)
	}

	/// Look a student number up as written.
	pub fn lookup_number(&self, number: &str) -> Option<usize> {
		self.lookup(&StudentKey::Number(normalize_key(number)))
	}

	/// Anything that makes this roster untrustworthy to grade from.
	pub fn errors(&self) -> impl Iterator<Item = &InputDiagnostic> {
		self.diagnostics
			.iter()
			.filter(|d| d.severity == DiagnosticSeverity::Error)
	}

	pub fn name_of(&self, key: &StudentKey) -> Option<&str> {
		self.lookup(key)
			.and_then(|i| self.entries[i].name.as_deref())
	}
}

/// What makes two rows for one key irreconcilable, if anything.
///
/// Only non-empty values count: a row that simply does not know someone's name does not
/// contradict one that does.
fn contradiction(kept: &RosterEntry, other: &RosterEntry) -> Option<String> {
	match (kept.name.as_deref(), other.name.as_deref()) {
		(Some(a), Some(b)) if a != b => return Some(format!("named '{a}' and '{b}'")),
		_ => {}
	}
	match (kept.canvas_user_id, other.canvas_user_id) {
		(Some(a), Some(b)) if a != b => Some(format!("Canvas ids {a} and {b}")),
		_ => None,
	}
}

/// Collapse rows that share a key.
///
/// A student number identifies one person, so repeated rows are the same person listed
/// twice — merged, and reported so the file gets cleaned up. Rows that disagree about who
/// that person is are a different matter: there is no answer to pick, so they are an
/// `Error` and the run stops rather than attributing someone's work to the wrong name.
///
/// Rows from *different* sources are never in conflict: Canvas spelling a name differently
/// from the teacher's spreadsheet is ordinary, and enrollment wins because it comes first.
fn merge_by_key(entries: Vec<RosterEntry>) -> (Vec<RosterEntry>, Vec<InputDiagnostic>) {
	let mut merged: Vec<RosterEntry> = Vec::new();
	let mut first_seen: std::collections::BTreeMap<StudentKey, usize> = Default::default();
	let mut repeats: std::collections::BTreeMap<StudentKey, usize> = Default::default();
	let mut conflicted: std::collections::BTreeSet<StudentKey> = Default::default();
	let mut diagnostics = Vec::new();

	for entry in entries {
		let Some(&i) = first_seen.get(&entry.key) else {
			first_seen.insert(entry.key.clone(), merged.len());
			merged.push(entry);
			continue;
		};

		let kept = &mut merged[i];
		if kept.source == entry.source
			&& let Some(detail) = contradiction(kept, &entry)
		{
			if conflicted.insert(entry.key.clone()) {
				let mut diagnostic =
					InputDiagnostic::error(DiagnosticKind::ConflictingRosterEntry {
						key: entry.key.to_string(),
						detail,
					});
				diagnostic.location = entry.location.clone();
				diagnostics.push(diagnostic);
			}
			continue;
		}

		*repeats.entry(entry.key.clone()).or_insert(1) += 1;
		// Fill in only what the kept row does not already know.
		if kept.name.is_none() {
			kept.name = entry.name;
		}
		if kept.canvas_user_id.is_none() {
			kept.canvas_user_id = entry.canvas_user_id;
		}
	}

	for (key, count) in repeats {
		if conflicted.contains(&key) {
			continue;
		}
		diagnostics.push(InputDiagnostic::warning(
			DiagnosticKind::DuplicateRosterEntry {
				key: key.to_string(),
				count,
			},
		));
	}

	(merged, diagnostics)
}

/// Load a roster CSV.
///
/// Expected format: `name,_,student_id` (header row skipped), or `name,student_id`.
/// Handles a UTF-8 BOM. Column *mapping* — choosing which column is which — is P-672;
/// this stays positional on purpose.
///
/// A row that parses but carries no usable student number is reported rather than skipped:
/// dropping it silently would take that student out of the roster of record, and with them
/// the `NotSubmitted` entry the model exists to preserve.
pub fn load_roster(path: &Path) -> Result<Roster, RosterError> {
	let content =
		std::fs::read_to_string(path).map_err(|e| RosterError::IoError(path.to_path_buf(), e))?;

	// Strip UTF-8 BOM if present
	let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

	let mut reader = csv::ReaderBuilder::new()
		.has_headers(true)
		.flexible(true)
		.from_reader(content.as_bytes());

	// The width the file declares. A row that does not match it has shifted — usually an
	// unescaped separator in a name — and its columns no longer mean what their position
	// says, so it must not be read positionally.
	let header_len = match reader.headers() {
		Ok(header) => header.len(),
		Err(e) => return Err(RosterError::CsvError(path.to_path_buf(), e)),
	};

	let mut entries = Vec::new();
	let mut diagnostics = Vec::new();

	for (row, result) in reader.records().enumerate() {
		let record = result.map_err(|e| RosterError::CsvError(path.to_path_buf(), e))?;
		// +2: one for the skipped header, one for 1-based line numbers.
		let location = SourceLocation::row(path.to_path_buf(), row + 2);

		// Format: name, _, student_id (or name, student_id)
		let name = record.get(0).unwrap_or("").trim().to_string();
		let student_number = if record.len() >= 3 {
			record.get(2).unwrap_or("")
		} else if record.len() >= 2 {
			record.get(1).unwrap_or("")
		} else {
			diagnostics.push(
				InputDiagnostic::warning(DiagnosticKind::UnusableRosterRow {
					reason: format!("only {} column(s); need at least 2", record.len()),
				})
				.at(location),
			);
			continue;
		};

		let student_number = normalize_key(student_number);
		if let Some(prefix) = crate::models::RESERVED_KEY_PREFIXES
			.iter()
			.find(|p| student_number.starts_with(**p))
		{
			diagnostics.push(
				InputDiagnostic::warning(DiagnosticKind::UnusableRosterRow {
					reason: format!(
						"student id '{student_number}' starts with the reserved prefix '{prefix}'"
					),
				})
				.at(location),
			);
			continue;
		}

		// Column 3 is the Canvas user id, which `roster-pull` now writes so that grade push
		// never has to guess it. Read for every row, independently of the key decision
		// below: a student with both a 学号 and a Canvas id needs both kept.
		let canvas_user_id = record
			.get(3)
			.map(normalize_key)
			.filter(|value| !value.is_empty())
			.and_then(|value| value.parse::<u64>().ok());

		if student_number.is_empty() {
			// A Canvas enrollee with no SIS id is still a member. They are keyed by their
			// Canvas id — but only from a row whose width matches the header, because a
			// shifted row puts somebody's 学号 in this column, and keying on it would
			// invent a Canvas user that does not exist.
			if let Some(id) = canvas_user_id.filter(|_| record.len() == header_len) {
				entries.push(RosterEntry {
					key: StudentKey::CanvasUser(id),
					source: RosterSource::Supplied,
					name: (!name.is_empty()).then_some(name),
					canvas_user_id: Some(id),
					location: Some(location),
				});
				continue;
			}

			diagnostics.push(
				InputDiagnostic::warning(DiagnosticKind::UnusableRosterRow {
					reason: if name.is_empty() {
						"blank row".to_string()
					} else if record.len() != header_len {
						format!(
							"'{name}' has no student id, and the row has {} column(s) where the \
							 header declares {header_len} — it has probably shifted",
							record.len()
						)
					} else {
						format!("'{name}' has no student id")
					},
				})
				.at(location),
			);
			continue;
		}

		entries.push(RosterEntry {
			key: StudentKey::Number(student_number),
			source: RosterSource::Supplied,
			name: (!name.is_empty()).then_some(name),
			canvas_user_id,
			location: Some(location),
		});
	}

	Ok(Roster::with_diagnostics(entries, diagnostics))
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
		assert_eq!(roster.lookup_number("alice123"), Some(0));
		assert_eq!(roster.entries[0].name.as_deref(), Some("Alice"));
		assert_eq!(roster.entries[1].name.as_deref(), Some("Bob"));
		assert!(roster.diagnostics.is_empty());
	}

	#[test]
	fn test_load_roster_with_bom() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(&dir, "\u{feff}name,class,student_id\nAlice,A,alice123\n");

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.entries[0].student_number(), Some("alice123"));
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
		assert_eq!(roster.lookup_number("0024010003"), Some(0));
		assert_eq!(roster.lookup_number("24010003"), Some(1));
		// Exact-text keys cannot collide, so this is not a duplicate.
		assert!(roster.diagnostics.is_empty());
	}

	#[test]
	fn test_repeated_identical_rows_are_merged_and_reported() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nAlice,A,2024010001\nAlice,B,2024010001\n",
		);

		let roster = load_roster(&path).unwrap();
		// One student number is one person, so a repeated row is that person listed twice.
		assert_eq!(roster.len(), 1);
		assert_eq!(roster.entries[0].name.as_deref(), Some("Alice"));
		assert_eq!(roster.diagnostics.len(), 1);
		assert_eq!(roster.diagnostics[0].severity, DiagnosticSeverity::Warning);
		assert!(matches!(
			&roster.diagnostics[0].kind,
			DiagnosticKind::DuplicateRosterEntry { key, count }
				if key == "2024010001" && *count == 2
		));
	}

	#[test]
	fn test_a_row_that_only_knows_less_is_not_a_conflict() {
		let dir = tempfile::tempdir().unwrap();
		// The second row has no name — it does not contradict the first, it just knows
		// less, so the two merge.
		let path = write(
			&dir,
			"name,class,student_id\n,A,2024010001\nAlice,B,2024010001\n",
		);

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.len(), 1);
		assert_eq!(roster.entries[0].name.as_deref(), Some("Alice"));
		assert!(roster.errors().next().is_none());
	}

	#[test]
	fn test_rows_that_disagree_about_who_a_number_is_are_an_error() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nAlice Wu,A,2024010001\nAlice Chen,B,2024010001\n",
		);

		let roster = load_roster(&path).unwrap();
		// There is no answer to pick between them, so this is not a warning to skim past.
		let errors: Vec<_> = roster.errors().collect();
		assert_eq!(errors.len(), 1);
		assert!(matches!(
			&errors[0].kind,
			DiagnosticKind::ConflictingRosterEntry { key, .. } if key == "2024010001"
		));
		// The line the clash was spotted on is named, so the file can be fixed.
		assert_eq!(errors[0].location.as_ref().and_then(|l| l.row), Some(3));
	}

	#[test]
	fn test_lookup_trims_but_does_not_otherwise_normalise() {
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		assert_eq!(roster.lookup_number(" 2024010001 "), Some(0));
		assert_eq!(roster.lookup_number("02024010001"), None);
		assert_eq!(roster.lookup_number("missing"), None);
	}

	#[test]
	fn test_an_unconfirmed_local_token_matches_a_student_number_row() {
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		assert_eq!(
			roster.lookup(&StudentKey::Extracted("2024010001".into())),
			Some(0)
		);
		// A Canvas id is a separate namespace and must not match a 学号 row.
		assert_eq!(roster.lookup(&StudentKey::CanvasUser(2024010001)), None);
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

	#[test]
	fn test_unusable_rows_are_reported_rather_than_dropped_silently() {
		let dir = tempfile::tempdir().unwrap();
		let path = write(
			&dir,
			"name,class,student_id\nBob Lin,,2024010002\nCarol,,\nsolo\n",
		);

		let roster = load_roster(&path).unwrap();
		assert_eq!(roster.len(), 1);
		// Carol's blank id and the one-column row each leave a trace naming their line.
		assert_eq!(roster.diagnostics.len(), 2);
		assert!(
			roster
				.diagnostics
				.iter()
				.all(|d| matches!(&d.kind, DiagnosticKind::UnusableRosterRow { .. }))
		);
		let rows: Vec<Option<usize>> = roster
			.diagnostics
			.iter()
			.map(|d| d.location.as_ref().and_then(|l| l.row))
			.collect();
		assert_eq!(rows, vec![Some(3), Some(4)]);
	}
}
