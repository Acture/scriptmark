use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{DiagnosticKind, InputDiagnostic, SourceLocation, StudentKey, normalize_key};

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

/// What a key lookup found. Duplicates make "the" matching entry a question with no single
/// answer, so the caller is forced to decide rather than silently taking the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterLookup {
	Unique(usize),
	Ambiguous(Vec<usize>),
	Missing,
}

impl RosterLookup {
	/// Every matching row; empty when there was no match.
	pub fn hits(&self) -> Vec<usize> {
		match self {
			Self::Unique(i) => vec![*i],
			Self::Ambiguous(hits) => hits.clone(),
			Self::Missing => Vec::new(),
		}
	}
}

impl Roster {
	pub fn from_entries(entries: Vec<RosterEntry>) -> Self {
		Self::with_diagnostics(entries, Vec::new())
	}

	pub fn with_diagnostics(
		entries: Vec<RosterEntry>,
		mut diagnostics: Vec<InputDiagnostic>,
	) -> Self {
		diagnostics.extend(duplicate_diagnostics(&entries));
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

	/// Exact-key lookup.
	///
	/// A student number is compared as written, after the same trim the loader applies —
	/// never case-folded, never zero-stripped. An unconfirmed local token and a confirmed
	/// 学号 denote the same thing, so they match the same row; a Canvas id is a separate
	/// namespace and only ever matches a Canvas-keyed row.
	pub fn lookup(&self, key: &StudentKey) -> RosterLookup {
		let matches = |entry: &RosterEntry| match (&entry.key, key) {
			(StudentKey::CanvasUser(a), StudentKey::CanvasUser(b)) => a == b,
			(StudentKey::CanvasUser(_), _) | (_, StudentKey::CanvasUser(_)) => false,
			(a, b) => a.raw() == b.raw(),
		};

		let hits: Vec<usize> = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| matches(entry))
			.map(|(i, _)| i)
			.collect();

		match hits.len() {
			0 => RosterLookup::Missing,
			1 => RosterLookup::Unique(hits[0]),
			_ => RosterLookup::Ambiguous(hits),
		}
	}

	/// Look a student number up as written.
	pub fn lookup_number(&self, number: &str) -> RosterLookup {
		self.lookup(&StudentKey::Number(normalize_key(number)))
	}

	/// The name on a row, when there is exactly one row for that key.
	pub fn name_of(&self, key: &StudentKey) -> Option<&str> {
		match self.lookup(key) {
			RosterLookup::Unique(i) => self.entries[i].name.as_deref(),
			_ => None,
		}
	}
}

fn duplicate_diagnostics(entries: &[RosterEntry]) -> Vec<InputDiagnostic> {
	// Counted by key value rather than by its rendering: the `Display` prefixes are not
	// escaped, so two different keys can render alike.
	let mut counts: std::collections::BTreeMap<&StudentKey, usize> = Default::default();
	for entry in entries {
		*counts.entry(&entry.key).or_default() += 1;
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

/// Prefixes [`StudentKey`] uses to mark an unconfirmed or Canvas-native key. A 学号 may not
/// begin with one, or the rendering would stop being reversible.
const RESERVED_PREFIXES: [&str; 2] = ["local:", "canvas:"];

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
		if let Some(prefix) = RESERVED_PREFIXES
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
		if student_number.is_empty() {
			diagnostics.push(
				InputDiagnostic::warning(DiagnosticKind::UnusableRosterRow {
					reason: if name.is_empty() {
						"blank row".to_string()
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
			canvas_user_id: None,
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
		assert_eq!(roster.lookup_number("alice123"), RosterLookup::Unique(0));
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
		assert_eq!(roster.lookup_number("0024010003"), RosterLookup::Unique(0));
		assert_eq!(roster.lookup_number("24010003"), RosterLookup::Unique(1));
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
			DiagnosticKind::DuplicateRosterEntry { key, count }
				if key == "2024010001" && *count == 2
		));

		assert_eq!(
			roster.lookup_number("2024010001"),
			RosterLookup::Ambiguous(vec![0, 1])
		);
		// An ambiguous key has no single name, and the loader does not invent one.
		assert_eq!(
			roster.name_of(&StudentKey::Number("2024010001".into())),
			None
		);
	}

	#[test]
	fn test_lookup_trims_but_does_not_otherwise_normalise() {
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		assert_eq!(
			roster.lookup_number(" 2024010001 "),
			RosterLookup::Unique(0)
		);
		assert_eq!(roster.lookup_number("02024010001"), RosterLookup::Missing);
		assert_eq!(roster.lookup_number("missing"), RosterLookup::Missing);
	}

	#[test]
	fn test_an_unconfirmed_local_token_matches_a_student_number_row() {
		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		assert_eq!(
			roster.lookup(&StudentKey::Extracted("2024010001".into())),
			RosterLookup::Unique(0)
		);
		// A Canvas id is a separate namespace and must not match a 学号 row.
		assert_eq!(
			roster.lookup(&StudentKey::CanvasUser(2024010001)),
			RosterLookup::Missing
		);
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
