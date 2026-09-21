//! The local entry point: scan directories of student files and produce an
//! [`AssignmentInput`].
//!
//! Every file that arrives is accounted for. A file whose owner can be identified but
//! whose type nothing runs becomes an `IgnoredFile` diagnostic against that student; a
//! file with no identifiable owner becomes an [`UnmatchedArtifact`]. Nothing is dropped on
//! the floor, and nothing is printed — anomalies are returned as diagnostics for the CLI
//! to render.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::models::{
	Assignment, AssignmentInput, AttemptPolicy, DiagnosticKind, FileOrigin, InputDiagnostic,
	InputSource, RosterMatch, SourceLocation, StudentFile, StudentIdentity, StudentKey,
	StudentSubmission, SubmissionAttempt, UnmatchedArtifact, UnmatchedReason, normalize_key,
};
use crate::roster::{Roster, RosterLookup};

/// Directory archives are expanded into, beside the directory being scanned.
const EXTRACT_DIR: &str = ".scriptmark_extracted";

/// Map file extensions to language identifiers.
///
/// Shared with the Canvas adapter, which classifies downloaded attachments the same way.
pub(crate) fn detect_language(ext: &str) -> Option<&'static str> {
	match ext {
		"py" => Some("python"),
		"cpp" | "cc" | "cxx" => Some("cpp"),
		"c" => Some("c"),
		"java" => Some("java"),
		"js" => Some("javascript"),
		"ts" => Some("typescript"),
		"rs" => Some("rust"),
		"go" => Some("go"),
		_ => None,
	}
}

/// Extract a student key from a filename.
///
/// Convention: `{key}_{rest}.ext` (e.g. `alice_Lab5_1.py` → `alice`). This split is a
/// placeholder — teacher-configurable matching is P-673 — so the key it yields is only
/// ever an unconfirmed [`crate::models::StudentKey::Extracted`] until a roster vouches
/// for it.
fn extract_sid(filename: &str) -> Option<String> {
	let stem = Path::new(filename).file_stem()?.to_str()?;
	// Normalised here rather than at each use: `seen_keys`, `by_key` and the roster-merge
	// coverage set are all keyed on this string, and a token with stray whitespace would
	// otherwise group separately from the identity built out of it.
	let sid = normalize_key(stem.split('_').next()?);
	if sid.is_empty() {
		return None;
	}
	Some(sid)
}

/// A file that came out of an archive, with the provenance needed to trace it back.
#[derive(Debug, Clone)]
struct ExtractedFile {
	out_path: PathBuf,
	archive: PathBuf,
	/// The path *inside* the archive, before flattening.
	entry: String,
}

const MAX_FILE_SIZE: u64 = 5_000_000; // 5 MB per file
const MAX_TOTAL_SIZE: u64 = 50_000_000; // 50 MB total per archive
const MAX_FILE_COUNT: usize = 100;

fn is_noise(name: &str) -> bool {
	name.starts_with('.') || name.starts_with("__")
}

fn skipped(archive: &Path, entry: &str, reason: String) -> InputDiagnostic {
	InputDiagnostic::warning(DiagnosticKind::ArchiveEntrySkipped {
		archive: archive.to_path_buf(),
		entry: entry.to_string(),
		reason,
	})
}

/// Extract `.zip` archives in a directory to `{EXTRACT_DIR}/{archive_stem}/`.
///
/// Archives already extracted are not re-extracted, but their index is re-read so that
/// provenance survives a second run — otherwise a cached extraction would leave every file
/// it produced with no traceable origin.
fn extract_archives(dir: &Path, diagnostics: &mut Vec<InputDiagnostic>) -> Vec<ExtractedFile> {
	let extract_root = dir.join(EXTRACT_DIR);
	let mut extracted = Vec::new();

	let Ok(entries) = std::fs::read_dir(dir) else {
		return extracted;
	};

	// Sort: read_dir order is not stable across filesystems.
	let mut archives: Vec<PathBuf> = entries
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			p.is_file()
				&& p.extension()
					.and_then(|e| e.to_str())
					.is_some_and(|e| e.eq_ignore_ascii_case("zip"))
		})
		.collect();
	archives.sort();

	for archive_path in archives {
		let stem = archive_path
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or("unknown");
		let target = extract_root.join(stem);

		let file = match std::fs::File::open(&archive_path) {
			Ok(f) => f,
			Err(e) => {
				diagnostics.push(InputDiagnostic::warning(
					DiagnosticKind::ArchiveUnreadable {
						archive: archive_path.clone(),
						reason: e.to_string(),
					},
				));
				continue;
			}
		};
		let mut archive = match zip::ZipArchive::new(file) {
			Ok(a) => a,
			Err(e) => {
				diagnostics.push(InputDiagnostic::warning(
					DiagnosticKind::ArchiveUnreadable {
						archive: archive_path.clone(),
						reason: e.to_string(),
					},
				));
				continue;
			}
		};

		if let Err(e) = std::fs::create_dir_all(&target) {
			diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::ArchiveUnreadable {
					archive: archive_path.clone(),
					reason: format!("cannot create extraction directory: {e}"),
				},
			));
			continue;
		}

		let mut total_bytes: u64 = 0;
		let mut file_count: usize = 0;
		// Flattening can map two in-archive paths onto one output name; remember who got
		// there first so the loser is reported rather than silently dropped.
		let mut claimed: BTreeMap<PathBuf, String> = BTreeMap::new();

		for i in 0..archive.len() {
			let mut entry = match archive.by_index(i) {
				Ok(e) => e,
				Err(_) => continue,
			};
			if entry.is_dir() {
				continue;
			}

			let Some(name) = entry.enclosed_name() else {
				// Path traversal attempt.
				diagnostics.push(InputDiagnostic::warning(
					DiagnosticKind::ArchiveEntrySkipped {
						archive: archive_path.clone(),
						entry: entry.name().to_string(),
						reason: "unsafe path".to_string(),
					},
				));
				continue;
			};
			let entry_name = name.to_string_lossy().into_owned();

			let Some(filename) = name.file_name().map(|n| n.to_owned()) else {
				continue;
			};
			if is_noise(&filename.to_string_lossy()) {
				continue;
			}

			let out_path = target.join(&filename);

			if let Some(first) = claimed.get(&out_path) {
				diagnostics.push(InputDiagnostic::warning(
					DiagnosticKind::ArchiveNameCollision {
						archive: archive_path.clone(),
						entry: format!("{entry_name} (already taken by {first})"),
					},
				));
				continue;
			}

			// Every guard runs before the entry is recorded. Claiming the name first would
			// let a rejected entry block the real submission from ever being extracted, and
			// the accounting runs on a cached rerun too so the diagnostics do not vanish
			// the second time a directory is scanned.
			if entry.size() > MAX_FILE_SIZE {
				diagnostics.push(skipped(
					&archive_path,
					&entry_name,
					format!(
						"{} bytes exceeds the {MAX_FILE_SIZE} byte limit",
						entry.size()
					),
				));
				continue;
			}
			if total_bytes + entry.size() > MAX_TOTAL_SIZE {
				diagnostics.push(skipped(
					&archive_path,
					&entry_name,
					format!("archive exceeds the {MAX_TOTAL_SIZE} byte total"),
				));
				break;
			}
			if file_count >= MAX_FILE_COUNT {
				diagnostics.push(skipped(
					&archive_path,
					&entry_name,
					format!("archive exceeds the {MAX_FILE_COUNT} file limit"),
				));
				break;
			}

			total_bytes += entry.size();
			file_count += 1;
			claimed.insert(out_path.clone(), entry_name.clone());

			// Provenance is recorded whether or not the bytes are written this run, so a
			// cached extraction still traces back to its archive entry.
			extracted.push(ExtractedFile {
				out_path: out_path.clone(),
				archive: archive_path.clone(),
				entry: entry_name.clone(),
			});

			// Only the bytes are skipped when the file is already there — an entry that
			// failed last run is retried, so its diagnostic recurs instead of vanishing on
			// the second scan of a directory.
			if out_path.exists() {
				continue;
			}

			let mut buf = Vec::new();
			let failure = match entry.read_to_end(&mut buf) {
				Err(_) => Some("unreadable entry".to_string()),
				Ok(_) => std::fs::write(&out_path, &buf).err().map(|e| e.to_string()),
			};
			if let Some(reason) = failure {
				diagnostics.push(skipped(&archive_path, &entry_name, reason));
				// Roll the claim and the provenance back together; letting them drift is
				// what lets a rejected entry block a real one.
				extracted.pop();
				claimed.remove(&out_path);
				total_bytes -= entry.size();
				file_count -= 1;
			}
		}
	}

	extracted
}

/// How to interpret a set of scanned directories.
pub struct LocalInputOptions<'a> {
	pub assignment: Assignment,
	/// The roster of record. Without one, membership is unknown rather than negative.
	pub roster: Option<&'a Roster>,
	pub attempt_policy: AttemptPolicy,
}

impl Default for LocalInputOptions<'_> {
	fn default() -> Self {
		Self {
			assignment: Assignment::default(),
			roster: None,
			attempt_policy: AttemptPolicy::Latest,
		}
	}
}

/// Scan directories of student submissions and build the unified input.
///
/// Local input has no notion of repeated attempts — inferring them from Canvas download
/// filename tokens would be a matching rule, which is P-673 — so every student gets
/// exactly one attempt.
pub fn load_local_input(
	paths: &[impl AsRef<Path>],
	options: LocalInputOptions<'_>,
) -> Result<AssignmentInput, DiscoveryError> {
	let mut diagnostics: Vec<InputDiagnostic> = Vec::new();
	let mut unmatched: Vec<UnmatchedArtifact> = Vec::new();
	// BTreeMap: iteration order must not depend on hashing.
	let mut by_key: BTreeMap<String, Vec<StudentFile>> = BTreeMap::new();
	// A student who sent only unusable files still submitted something.
	let mut seen_keys: std::collections::BTreeSet<String> = Default::default();

	for path in paths {
		let path = path.as_ref();
		if !path.is_dir() {
			return Err(DiscoveryError::NotADirectory(path.to_path_buf()));
		}
	}

	let mut origins: BTreeMap<PathBuf, FileOrigin> = BTreeMap::new();
	let mut extra_dirs: Vec<PathBuf> = Vec::new();
	for path in paths {
		for extracted in extract_archives(path.as_ref(), &mut diagnostics) {
			if let Some(parent) = extracted.out_path.parent()
				&& !extra_dirs.contains(&parent.to_path_buf())
			{
				extra_dirs.push(parent.to_path_buf());
			}
			origins.insert(
				extracted.out_path,
				FileOrigin::Archive {
					archive: extracted.archive,
					entry: extracted.entry,
				},
			);
		}
	}

	let all_paths: Vec<PathBuf> = paths
		.iter()
		.map(|p| p.as_ref().to_path_buf())
		.chain(extra_dirs)
		.collect();

	for dir_path in &all_paths {
		let entries = std::fs::read_dir(dir_path)
			.map_err(|e| DiscoveryError::IoError(dir_path.clone(), e))?;

		let mut files: Vec<PathBuf> = entries
			.filter_map(|e| e.ok())
			.map(|e| e.path())
			.filter(|p| p.is_file())
			.collect();
		files.sort();

		let is_extracted = dir_path.components().any(|c| c.as_os_str() == EXTRACT_DIR);

		for path in files {
			let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
				continue;
			};
			if is_noise(filename) {
				continue;
			}

			let ext = path
				.extension()
				.and_then(|e| e.to_str())
				.unwrap_or("")
				.to_lowercase();
			// Archives are inputs to extraction, not submissions in their own right — but
			// the upload still happened. Registering the owner here is what stops a
			// truncated or empty archive being reported as 缺交.
			if !is_extracted && ext == "zip" {
				match extract_sid(filename) {
					Some(key) => {
						seen_keys.insert(key);
					}
					None => unmatched.push(UnmatchedArtifact {
						path: path.clone(),
						reason: UnmatchedReason::NoStudentKey,
					}),
				}
				continue;
			}

			// Inside an extraction directory the archive stem carries the key; the files
			// within it are named by the student.
			let key = if is_extracted {
				dir_path
					.file_name()
					.and_then(|n| n.to_str())
					.and_then(extract_sid)
					.or_else(|| extract_sid(filename))
			} else {
				extract_sid(filename)
			};

			let language = detect_language(&ext);

			match (key, language) {
				(Some(key), Some(language)) => {
					seen_keys.insert(key.clone());
					let origin = origins.get(&path).cloned().unwrap_or(FileOrigin::Direct);
					by_key
						.entry(key)
						.or_default()
						.push(StudentFile::direct(path.clone(), language).with_origin(origin));
				}
				(Some(key), None) => {
					// Owner known, type unusable: the student submitted, just not code.
					diagnostics.push(
						InputDiagnostic::info(DiagnosticKind::IgnoredFile {
							key: key.clone(),
							path: path.clone(),
						})
						.at(SourceLocation::file(path.clone())),
					);
					seen_keys.insert(key);
				}
				(None, language) => unmatched.push(UnmatchedArtifact {
					path: path.clone(),
					reason: if language.is_some() {
						UnmatchedReason::NoStudentKey
					} else {
						UnmatchedReason::UnsupportedType
					},
				}),
			}
		}
	}

	let mut students: Vec<StudentSubmission> = Vec::new();
	// Keyed on the value, not its rendering: `Display` prefixes are not escaped, so a 学号
	// that literally reads `canvas:5` would otherwise collide with `CanvasUser(5)`.
	let mut covered: std::collections::BTreeSet<StudentKey> = Default::default();
	let mut covered_canvas_ids: std::collections::BTreeSet<u64> = Default::default();

	for key in &seen_keys {
		let mut files = by_key.remove(key).unwrap_or_default();
		files.sort_by(|a, b| a.path.cmp(&b.path));

		let mut identity = StudentIdentity::extracted(key);
		let roster_match = match options.roster {
			None => RosterMatch::NoRoster,
			Some(roster) => match roster.lookup(&identity.key) {
				RosterLookup::Unique(i) => {
					identity.confirm_number();
					identity.name = roster.entries[i].name.clone();
					identity.canvas_user_id = roster.entries[i].canvas_user_id;
					covered.insert(identity.key.clone());
					covered_canvas_ids.extend(identity.canvas_user_id);
					RosterMatch::Matched(i)
				}
				RosterLookup::Ambiguous(hits) => {
					identity.confirm_number();
					covered.insert(identity.key.clone());
					covered_canvas_ids.extend(identity.canvas_user_id);
					diagnostics.push(InputDiagnostic::warning(
						DiagnosticKind::AmbiguousRosterMatch {
							key: key.clone(),
							count: hits.len(),
						},
					));
					RosterMatch::Ambiguous(hits)
				}
				RosterLookup::Missing => {
					diagnostics.push(InputDiagnostic::warning(DiagnosticKind::NotOnRoster {
						key: key.clone(),
					}));
					RosterMatch::NotInRoster
				}
			},
		};

		let attempt = SubmissionAttempt::new(1).with_files(files);
		students.push(StudentSubmission::received(
			identity,
			roster_match,
			vec![attempt],
			options.attempt_policy,
		));
	}

	// A roster student who sent nothing must still appear — that is the whole point of
	// having a roster of record. Duplicate rows for one number yield one student carrying
	// every row it matched, not one student per row.
	if let Some(roster) = options.roster {
		for entry in &roster.entries {
			if !covered.insert(entry.key.clone()) {
				continue;
			}
			// A roster that names one person under both a 学号 and a Canvas id must not
			// count them twice.
			if entry
				.canvas_user_id
				.is_some_and(|id| covered_canvas_ids.contains(&id))
			{
				continue;
			}
			covered_canvas_ids.extend(entry.canvas_user_id);
			let hits = roster.lookup(&entry.key).hits();
			let mut identity = match &entry.key {
				StudentKey::CanvasUser(id) => StudentIdentity::canvas_user(*id),
				key => StudentIdentity::number(key.raw()),
			};
			identity.name = entry.name.clone();
			identity.canvas_user_id = identity.canvas_user_id.or(entry.canvas_user_id);
			students.push(StudentSubmission::not_submitted(identity, hits));
		}
	}

	let mut input = AssignmentInput {
		assignment: options.assignment,
		source: InputSource::Local {
			scanned_dirs: paths.iter().map(|p| p.as_ref().to_path_buf()).collect(),
			roster_path: options
				.roster
				.and_then(|r| r.entries.first())
				.and_then(|e| e.location.as_ref())
				.and_then(|l| l.file.clone()),
		},
		roster: options.roster.cloned(),
		students,
		unmatched,
		diagnostics,
	};

	if let Some(roster) = options.roster {
		input.diagnostics.extend(roster.diagnostics.iter().cloned());
	}
	let zero_padded = input.detect_zero_padded_variants();
	input.diagnostics.extend(zero_padded);

	Ok(input.sorted())
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
	#[error("not a directory: {0}")]
	NotADirectory(std::path::PathBuf),
	#[error("IO error reading {0}: {1}")]
	IoError(std::path::PathBuf, std::io::Error),
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{StudentKey, SubmissionOutcome};

	fn scan(dir: &Path) -> AssignmentInput {
		load_local_input(&[dir], LocalInputOptions::default()).unwrap()
	}

	fn scan_with(dir: &Path, roster: &Roster) -> AssignmentInput {
		load_local_input(
			&[dir],
			LocalInputOptions {
				roster: Some(roster),
				..Default::default()
			},
		)
		.unwrap()
	}

	#[test]
	fn test_extract_sid() {
		assert_eq!(extract_sid("alice_Lab5_1.py"), Some("alice".to_string()));
		assert_eq!(extract_sid("bob_hw3.py"), Some("bob".to_string()));
		assert_eq!(
			extract_sid("student123_final.cpp"),
			Some("student123".to_string())
		);
		assert_eq!(extract_sid("_invalid.py"), None);
	}

	#[test]
	fn test_detect_language() {
		assert_eq!(detect_language("py"), Some("python"));
		assert_eq!(detect_language("cpp"), Some("cpp"));
		assert_eq!(detect_language("java"), Some("java"));
		assert_eq!(detect_language("txt"), None);
	}

	#[test]
	fn test_discover_submissions() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("alice_Lab5.py"), "pass").unwrap();
		std::fs::write(dir.path().join("bob_Lab5.py"), "pass").unwrap();
		std::fs::write(dir.path().join("alice_Lab6.py"), "pass").unwrap();

		let input = scan(dir.path());
		assert_eq!(input.student_count(), 2);
		assert_eq!(input.students[0].files().len(), 2); // alice
		assert_eq!(input.students[1].files().len(), 1); // bob
		assert_eq!(input.languages(), vec!["python"]);
		// With no roster, membership is unknown rather than negative.
		assert!(
			input
				.students
				.iter()
				.all(|s| s.roster_match == RosterMatch::NoRoster)
		);
		assert_eq!(
			input.with_outcome(SubmissionOutcome::NotSubmitted).count(),
			0
		);
	}

	#[test]
	fn test_discover_extracts_zip_archives_and_records_provenance() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("alice_Lab5.py"), "pass").unwrap();

		let zip_path = dir.path().join("bob_12345_67890_Lab5.zip");
		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		zip.start_file("src/Lab5.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		use std::io::Write;
		zip.write_all(b"def foo(): return 42").unwrap();
		zip.finish().unwrap();

		let input = scan(dir.path());
		assert_eq!(input.student_count(), 2);

		let bob = input
			.students
			.iter()
			.find(|s| s.key().raw() == "bob")
			.expect("bob");
		// The in-archive path survives flattening.
		assert_eq!(
			bob.files()[0].origin,
			FileOrigin::Archive {
				archive: zip_path.clone(),
				entry: "src/Lab5.py".to_string(),
			}
		);

		// A second run reuses the extraction and must still report where the file came from.
		let again = scan(dir.path());
		let bob_again = again
			.students
			.iter()
			.find(|s| s.key().raw() == "bob")
			.expect("bob");
		assert_eq!(bob_again.files()[0].origin, bob.files()[0].origin);
	}

	#[test]
	fn test_archive_name_collision_is_reported_not_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let zip_path = dir.path().join("carol_Lab5.zip");
		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		use std::io::Write;
		for folder in ["a", "b"] {
			zip.start_file(
				format!("{folder}/Lab5.py"),
				zip::write::SimpleFileOptions::default(),
			)
			.unwrap();
			zip.write_all(b"pass").unwrap();
		}
		zip.finish().unwrap();

		let input = scan(dir.path());
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::ArchiveNameCollision { .. })),
			"expected a collision diagnostic, got {:?}",
			input.diagnostics
		);
	}

	#[test]
	fn test_unusable_file_makes_the_owner_submitted_empty() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("2024010005_notes.txt"), "hello").unwrap();

		let roster = Roster::from_pairs(&[("2024010005", "Eve")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 1);
		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::SubmittedEmpty
		);
		assert!(input.unmatched.is_empty());
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::IgnoredFile { .. }))
		);
	}

	#[test]
	fn test_keyless_file_becomes_an_unmatched_artifact() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("_notes_v2.py"), "pass").unwrap();

		let input = scan(dir.path());
		assert_eq!(input.student_count(), 0);
		assert_eq!(input.unmatched.len(), 1);
		assert_eq!(input.unmatched[0].reason, UnmatchedReason::NoStudentKey);
	}

	#[test]
	fn test_roster_students_who_did_not_submit_survive() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("2024010001_lab1.py"), "pass").unwrap();

		let roster = Roster::from_pairs(&[("2024010001", "Alice"), ("2024010004", "Dan")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 2);
		let dan = input
			.students
			.iter()
			.find(|s| s.key().raw() == "2024010004")
			.expect("the non-submitter must not disappear");
		assert_eq!(dan.outcome(), SubmissionOutcome::NotSubmitted);
		assert_eq!(dan.identity.name.as_deref(), Some("Dan"));
		assert_eq!(dan.roster_match, RosterMatch::Matched(1));
	}

	#[test]
	fn test_submitter_absent_from_roster_is_kept_as_unmatched() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("9999999999_lab1.py"), "pass").unwrap();

		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let input = scan_with(dir.path(), &roster);

		let stranger = input
			.students
			.iter()
			.find(|s| s.key().raw() == "9999999999")
			.expect("stranger");
		assert_eq!(stranger.outcome(), SubmissionOutcome::ReceivedUnmatched);
		// Unconfirmed by any roster, so the key stays a bare extracted token.
		assert!(matches!(stranger.key(), StudentKey::Extracted(_)));
		assert_eq!(stranger.identity.key.to_string(), "local:9999999999");
	}

	#[test]
	fn test_leading_zero_pair_stays_distinct_and_is_flagged() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("0024010003_lab1.py"), "pass").unwrap();
		std::fs::write(dir.path().join("24010003_lab1.py"), "pass").unwrap();

		let roster = Roster::from_pairs(&[("0024010003", "Carol"), ("24010003", "Dave")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 2);
		assert_eq!(input.students[0].identity.name.as_deref(), Some("Carol"));
		assert_eq!(input.students[1].identity.name.as_deref(), Some("Dave"));
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::SuspectedZeroPaddedVariant { .. }))
		);
	}

	#[test]
	fn test_duplicate_roster_rows_do_not_spawn_a_phantom_non_submitter() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("2024010001_lab1.py"), "pass").unwrap();

		let roster = Roster::from_pairs(&[("2024010001", "Alice"), ("2024010001", "Alice Chen")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 1);
		assert_eq!(
			input.students[0].roster_match,
			RosterMatch::Ambiguous(vec![0, 1])
		);
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
		assert!(input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::DuplicateRosterEntry { count, .. } if *count == 2
		)));
	}

	#[test]
	fn test_duplicate_roster_rows_do_not_fan_a_non_submitter_out() {
		let dir = tempfile::tempdir().unwrap();
		let roster = Roster::from_pairs(&[("2024010001", "Alice"), ("2024010001", "Alice Chen")]);
		let input = scan_with(dir.path(), &roster);

		// One student for one number, however many rows name them — two entries would
		// collide on student_id the moment anything tried to persist them.
		assert_eq!(input.student_count(), 1);
		assert_eq!(
			input.students[0].roster_match,
			RosterMatch::Ambiguous(vec![0, 1])
		);
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::NotSubmitted);
	}

	#[test]
	fn test_a_token_with_stray_whitespace_does_not_split_a_student_in_two() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("2024010001 _lab1.py"), "pass").unwrap();

		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 1);
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
		assert_eq!(input.students[0].roster_match, RosterMatch::Matched(0));
	}

	#[test]
	fn test_an_unreadable_archive_is_not_reported_as_a_non_submission() {
		let dir = tempfile::tempdir().unwrap();
		// A truncated upload: the name is intact, the bytes are not.
		std::fs::write(
			dir.path().join("2024010001_lab1.zip"),
			b"PK\x03\x04 truncated",
		)
		.unwrap();

		let roster = Roster::from_pairs(&[("2024010001", "Alice")]);
		let input = scan_with(dir.path(), &roster);

		assert_eq!(input.student_count(), 1);
		// Something arrived — it just could not be opened. That is not 缺交.
		assert_eq!(
			input.students[0].outcome(),
			SubmissionOutcome::SubmittedEmpty
		);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::ArchiveUnreadable { .. }))
		);
	}

	#[test]
	fn test_a_rejected_archive_entry_does_not_block_the_real_submission() {
		let dir = tempfile::tempdir().unwrap();
		let zip_path = dir.path().join("2024010001_lab1.zip");
		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		use std::io::Write;
		// Oversized junk that flattens onto the same name as the real file.
		zip.start_file("junk/lab1.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(&vec![b'#'; (MAX_FILE_SIZE + 1) as usize])
			.unwrap();
		zip.start_file("src/lab1.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(b"def f(): return 1").unwrap();
		zip.finish().unwrap();

		let input = scan(dir.path());

		// The rejected entry must not reserve the name the real submission needs.
		assert_eq!(input.student_count(), 1);
		assert_eq!(input.students[0].outcome(), SubmissionOutcome::Executable);
		assert_eq!(
			input.students[0].files()[0].origin,
			FileOrigin::Archive {
				archive: zip_path,
				entry: "src/lab1.py".to_string(),
			}
		);
	}

	#[test]
	fn test_skip_diagnostics_survive_a_cached_rerun() {
		let dir = tempfile::tempdir().unwrap();
		let zip_path = dir.path().join("2024010001_lab1.zip");
		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		use std::io::Write;
		zip.start_file("ok.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(b"pass").unwrap();
		zip.start_file("huge.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(&vec![b'#'; (MAX_FILE_SIZE + 1) as usize])
			.unwrap();
		zip.finish().unwrap();

		let skips = |input: &AssignmentInput| {
			input
				.diagnostics
				.iter()
				.filter(|d| matches!(&d.kind, DiagnosticKind::ArchiveEntrySkipped { .. }))
				.count()
		};
		// A teacher rerunning the same directory must still be told a file was dropped.
		assert_eq!(skips(&scan(dir.path())), 1);
		assert_eq!(skips(&scan(dir.path())), 1);
	}

	/// Builds a zip whose `bad.py` payload fails its CRC check, so `read_to_end` errors.
	fn zip_with_a_corrupt_entry(path: &std::path::Path) {
		use std::io::Write;
		let file = std::fs::File::create(path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		let stored = zip::write::SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Stored);
		zip.start_file("good.py", stored).unwrap();
		zip.write_all(b"pass").unwrap();
		zip.start_file("bad.py", stored).unwrap();
		zip.write_all(b"PAYLOAD!").unwrap();
		zip.finish().unwrap();

		// Flip a payload byte after the CRC was computed.
		let mut bytes = std::fs::read(path).unwrap();
		let at = bytes
			.windows(8)
			.position(|w| w == b"PAYLOAD!")
			.expect("payload");
		bytes[at] ^= 0xff;
		std::fs::write(path, bytes).unwrap();
	}

	#[test]
	fn test_an_unreadable_entry_is_reported_on_every_run_not_just_the_first() {
		let dir = tempfile::tempdir().unwrap();
		zip_with_a_corrupt_entry(&dir.path().join("2024010001_lab1.zip"));

		let skips = |input: &AssignmentInput| {
			input
				.diagnostics
				.iter()
				.filter(|d| matches!(&d.kind, DiagnosticKind::ArchiveEntrySkipped { .. }))
				.count()
		};
		// A teacher rerunning the same directory must still be told the file was dropped.
		assert_eq!(skips(&scan(dir.path())), 1);
		assert_eq!(skips(&scan(dir.path())), 1);
		assert_eq!(
			serde_json::to_string(&scan(dir.path())).unwrap(),
			serde_json::to_string(&scan(dir.path())).unwrap()
		);
	}

	#[test]
	fn test_one_students_skip_does_not_swallow_anothers() {
		let dir = tempfile::tempdir().unwrap();
		// Students name their files after the assignment, so entry names collide across
		// archives by construction — the diagnostic must be attributed per archive.
		zip_with_a_corrupt_entry(&dir.path().join("2024010001_lab1.zip"));
		zip_with_a_corrupt_entry(&dir.path().join("2024010002_lab1.zip"));

		let input = scan(dir.path());
		let archives: std::collections::BTreeSet<String> = input
			.diagnostics
			.iter()
			.filter_map(|d| match &d.kind {
				DiagnosticKind::ArchiveEntrySkipped { archive, .. } => {
					Some(archive.file_name()?.to_string_lossy().into_owned())
				}
				_ => None,
			})
			.collect();
		assert_eq!(
			archives.len(),
			2,
			"both students must be told, got {archives:?}"
		);
	}

	#[test]
	fn test_a_reserved_prefix_in_a_roster_id_does_not_swallow_a_student() {
		let dir = tempfile::tempdir().unwrap();
		let roster_path = dir.path().join("roster.csv");
		std::fs::write(
			&roster_path,
			"name,class,student_id\nAlice,A,2024010001\nOdd,B,canvas:5\n",
		)
		.unwrap();
		let roster = crate::roster::load_roster(&roster_path).unwrap();

		let subs = dir.path().join("subs");
		std::fs::create_dir(&subs).unwrap();
		let input = load_local_input(
			&[&subs],
			LocalInputOptions {
				roster: Some(&roster),
				..Default::default()
			},
		)
		.unwrap();

		// The reserved-prefix row is refused at the door and said so, rather than being
		// silently merged with a Canvas-keyed student later.
		assert_eq!(input.student_count(), 1);
		assert!(
			input
				.diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::UnusableRosterRow { .. }))
		);
	}

	#[test]
	fn test_output_is_byte_identical_across_runs() {
		let dir = tempfile::tempdir().unwrap();
		for i in 0..12 {
			std::fs::write(dir.path().join(format!("s{i:04}_lab1.py")), "pass").unwrap();
		}
		std::fs::write(dir.path().join("_orphan.py"), "pass").unwrap();
		std::fs::write(dir.path().join("s0003_notes.txt"), "hi").unwrap();

		let first = serde_json::to_string(&scan(dir.path())).unwrap();
		let second = serde_json::to_string(&scan(dir.path())).unwrap();
		assert_eq!(first, second);
	}

	#[test]
	fn test_not_a_directory_is_a_hard_error() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("a.py");
		std::fs::write(&file, "pass").unwrap();

		let err = load_local_input(&[&file], LocalInputOptions::default()).unwrap_err();
		assert!(matches!(err, DiscoveryError::NotADirectory(_)));
	}
}
