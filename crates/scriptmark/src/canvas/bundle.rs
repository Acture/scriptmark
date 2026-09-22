//! What a Canvas fetch leaves on disk, and how a later run reads it back.
//!
//! ```text
//! <bundle>/
//!   canvas-payload.json          exactly what was fetched
//!   attachments.json             id -> {path, size} | {error}
//!   attachments/<id>/<name>      one directory per attachment id
//! ```
//!
//! The bundle exists so grading can re-run offline: fixing a test spec must not re-download
//! a class's work, and a bundle captured from a real course *is* a `CanvasPayload` fixture.
//! The id directory is what keeps two students' `hw1.py` apart while preserving the name
//! each of them actually used.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::client::{CanvasClient, CanvasError};
use crate::discovery::expand_archive;
use crate::input::canvas::{
	CanvasAttachmentPayload, CanvasPayload, DownloadedAttachment, DownloadedAttachments,
	ExpandedEntry,
};
use crate::models::InputDiagnostic;

const PAYLOAD_FILE: &str = "canvas-payload.json";
const MANIFEST_FILE: &str = "attachments.json";
const ATTACHMENT_DIR: &str = "attachments";

/// What became of one attachment, as recorded in the bundle.
///
/// This records *delivery*, not content: an archive's expanded entries are deliberately
/// absent. Flattening loses the in-archive path, so only the zip index can restore it, and
/// a recorded second copy would drift from disk the moment an extraction directory is
/// touched. Expansion is re-derived on every load instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentRecord {
	Stored { path: PathBuf, size: u64 },
	Failed { error: String },
}

type Manifest = BTreeMap<String, AttachmentRecord>;

/// How a fetch is getting on, so the CLI can show progress. Adapters never print.
pub struct FetchProgress<'a> {
	pub done: usize,
	pub total: usize,
	pub filename: &'a str,
	pub skipped: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
	#[error(transparent)]
	Canvas(#[from] CanvasError),
	#[error("{path}: {source}")]
	Io {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	#[error("{path} is not readable as JSON: {source}")]
	Json {
		path: PathBuf,
		#[source]
		source: serde_json::Error,
	},
	#[error("{0} does not look like a Canvas bundle: no {PAYLOAD_FILE}")]
	NotABundle(PathBuf),
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> BundleError + '_ {
	move |source| BundleError::Io {
		path: path.to_path_buf(),
		source,
	}
}

/// The file name an attachment is stored under.
///
/// Canvas keeps `display_name` as raw user input with only a truncation — it refuses to put
/// it on a filesystem unsanitised, and so must we: a student who renames their upload to
/// `../../../../evil.py` gets that string back in the submissions JSON. Reduced to a single
/// path component, or replaced by a name derived from the id.
///
/// The writer and the offline loader both call this. Sanitising on write alone would leave
/// the loader deriving a different path from the same payload, missing every file.
pub fn disk_name(payload: &CanvasAttachmentPayload) -> String {
	let raw = payload.name();
	let candidate = Path::new(&raw).file_name().and_then(|n| n.to_str());
	match candidate {
		Some(name) if !name.is_empty() && name != "." && name != ".." => name.to_string(),
		_ => format!("attachment-{}", payload.id),
	}
}

fn attachment_path(root: &Path, payload: &CanvasAttachmentPayload) -> PathBuf {
	root.join(ATTACHMENT_DIR)
		.join(payload.id.to_string())
		.join(disk_name(payload))
}

/// Every distinct attachment the payload mentions, in id order.
///
/// Canvas repeats a carried-forward file in each later attempt, so walking attempts would
/// fetch the same bytes several times and report a file count no teacher would recognise.
fn distinct_attachments(payload: &CanvasPayload) -> BTreeMap<u64, &CanvasAttachmentPayload> {
	let mut found = BTreeMap::new();
	for submission in &payload.submissions {
		let rows = std::iter::once(submission).chain(submission.submission_history.iter());
		for row in rows {
			for attachment in &row.attachments {
				found.entry(attachment.id).or_insert(attachment);
			}
		}
	}
	found
}

fn is_zip(path: &Path) -> bool {
	path.extension()
		.and_then(|e| e.to_str())
		.is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// Expand an archive attachment beside itself, and map it into the input's shape.
fn expansion_of(path: &Path, diagnostics: &mut Vec<InputDiagnostic>) -> Vec<ExpandedEntry> {
	if !is_zip(path) {
		return Vec::new();
	}
	let stem = path
		.file_stem()
		.and_then(|s| s.to_str())
		.unwrap_or("archive");
	let target = path.with_file_name(format!("{stem}.extracted"));
	expand_archive(path, &target, diagnostics)
		.into_iter()
		.map(|file| ExpandedEntry {
			entry: file.entry,
			path: file.out_path,
		})
		.collect()
}

/// Fetch a course assignment into `root`, returning the payload that was saved.
///
/// Listings are fetched first and are fatal on failure: the cohort is the thing being
/// established, so a submissions page that quietly went missing would become a class of
/// students who look like they never handed anything in. Individual attachments are the
/// opposite — a refusal is recorded and the fetch carries on.
///
/// Nothing that already exists is destroyed on the way: attachments are staged per file,
/// and the two JSON files are written last, so a run that dies partway leaves the previous
/// bundle's manifest and files intact.
pub async fn fetch(
	client: Arc<CanvasClient>,
	course_id: u64,
	assignment_id: u64,
	root: &Path,
	concurrency: usize,
	mut progress: impl FnMut(FetchProgress<'_>),
) -> Result<CanvasPayload, BundleError> {
	let users = client.pull_roster(course_id).await?;
	let assignment = client.fetch_assignment(course_id, assignment_id).await?;
	let submissions = client.fetch_submissions(course_id, assignment_id).await?;

	let payload = CanvasPayload {
		course_id: Some(course_id),
		assignment_id: Some(assignment_id),
		assignment_name: assignment.name.clone(),
		users,
		submissions,
	};

	std::fs::create_dir_all(root).map_err(io(root))?;

	let wanted = distinct_attachments(&payload);
	let total = wanted.len();
	let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
	let mut tasks = tokio::task::JoinSet::new();

	for (id, attachment) in &wanted {
		let dest = attachment_path(root, attachment);
		let url = attachment.url.clone();
		let expected = attachment.size;
		let name = disk_name(attachment);
		let client = Arc::clone(&client);
		let semaphore = Arc::clone(&semaphore);
		let id = *id;

		tasks.spawn(async move {
			let _permit = semaphore.acquire_owned().await;

			// Skipping on a matching size is what makes re-fetching cheap. A missing size
			// is not treated as agreement: guessing that an existing byte count is complete
			// is how a truncated file becomes a permanent zero.
			if let Some(size) = expected
				&& std::fs::metadata(&dest).is_ok_and(|m| m.len() == size)
			{
				return (id, name, Ok(dest), true);
			}

			let Some(url) = url else {
				return (
					id,
					name,
					Err("Canvas gave the attachment no download url".to_string()),
					false,
				);
			};

			let outcome = client
				.download_attachment(&url, &dest)
				.await
				.map(|_| dest)
				.map_err(|e| e.to_string());
			(id, name, outcome, false)
		});
	}

	let mut manifest: Manifest = Manifest::new();
	let mut done = 0usize;
	while let Some(joined) = tasks.join_next().await {
		let (id, name, outcome, skipped) = match joined {
			Ok(result) => result,
			// A panicked download task is a failed attachment, not a lost student.
			Err(e) => {
				done += 1;
				progress(FetchProgress {
					done,
					total,
					filename: "<panicked>",
					skipped: false,
				});
				manifest.insert(
					format!("join-error-{e}"),
					AttachmentRecord::Failed {
						error: e.to_string(),
					},
				);
				continue;
			}
		};
		done += 1;
		progress(FetchProgress {
			done,
			total,
			filename: &name,
			skipped,
		});

		let record = match outcome {
			Ok(path) => {
				let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
				AttachmentRecord::Stored { path, size }
			}
			Err(error) => AttachmentRecord::Failed { error },
		};
		manifest.insert(id.to_string(), record);
	}

	// Written last, and atomically: a failure above leaves the previous bundle readable.
	write_json(&root.join(PAYLOAD_FILE), &payload)?;
	write_json(&root.join(MANIFEST_FILE), &manifest)?;

	Ok(payload)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), BundleError> {
	let json = serde_json::to_string_pretty(value).map_err(|source| BundleError::Json {
		path: path.to_path_buf(),
		source,
	})?;
	let staging = path.with_extension("part");
	std::fs::write(&staging, json).map_err(io(&staging))?;
	std::fs::rename(&staging, path).map_err(io(path))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, BundleError> {
	let raw = std::fs::read_to_string(path).map_err(io(path))?;
	serde_json::from_str(&raw).map_err(|source| BundleError::Json {
		path: path.to_path_buf(),
		source,
	})
}

/// Read a bundle back, ready for `normalize`.
///
/// The manifest is checked against the disk rather than trusted: a bundle that has been
/// partly deleted yields `AttachmentUnavailable` here, where it names the student, instead
/// of a file-not-found much later when something tries to run it.
pub fn load(
	root: &Path,
) -> Result<(CanvasPayload, DownloadedAttachments, Vec<InputDiagnostic>), BundleError> {
	let payload_path = root.join(PAYLOAD_FILE);
	if !payload_path.is_file() {
		return Err(BundleError::NotABundle(root.to_path_buf()));
	}
	let payload: CanvasPayload = read_json(&payload_path)?;

	let manifest_path = root.join(MANIFEST_FILE);
	let manifest: Manifest = if manifest_path.is_file() {
		read_json(&manifest_path)?
	} else {
		Manifest::new()
	};

	let mut diagnostics = Vec::new();
	let mut downloads = DownloadedAttachments::new();
	let mut recorded: BTreeSet<u64> = BTreeSet::new();

	for (id, record) in &manifest {
		let Ok(id) = id.parse::<u64>() else {
			continue;
		};
		recorded.insert(id);
		let value = match record {
			AttachmentRecord::Failed { error } => Err(error.clone()),
			AttachmentRecord::Stored { path, .. } if !path.is_file() => Err(format!(
				"recorded in {MANIFEST_FILE} but missing from the bundle: {}",
				path.display()
			)),
			AttachmentRecord::Stored { path, .. } => Ok(DownloadedAttachment {
				expanded: expansion_of(path, &mut diagnostics),
				path: path.clone(),
			}),
		};
		downloads.insert(id, value);
	}

	// An attachment the payload mentions but the manifest never recorded was never
	// attempted — an interrupted fetch. Left absent, which is what `PendingDownload` means.
	Ok((payload, downloads, diagnostics))
}

/// Where the normalised input is written, so the record of which attempt was graded, and
/// of every file's provenance, outlives the process that produced it.
pub fn input_path(root: &Path) -> PathBuf {
	root.join("input.json")
}

pub fn save_input(root: &Path, input: &crate::models::AssignmentInput) -> Result<(), BundleError> {
	write_json(&input_path(root), input)
}

/// Fold the bundle's own diagnostics into a normalised input, keeping its ordering
/// invariant intact.
pub fn merge_diagnostics(
	mut input: crate::models::AssignmentInput,
	extra: Vec<InputDiagnostic>,
) -> crate::models::AssignmentInput {
	input.diagnostics.extend(extra);
	input.diagnostics.sort();
	input.diagnostics.dedup();
	input
}

#[cfg(test)]
mod tests {
	use super::*;

	fn attachment(id: u64, name: &str) -> CanvasAttachmentPayload {
		CanvasAttachmentPayload {
			id,
			filename: Some(name.to_string()),
			display_name: Some(name.to_string()),
			content_type: None,
			size: None,
			url: None,
		}
	}

	#[test]
	fn test_disk_name_refuses_a_traversal() {
		let evil = attachment(7, "../../../../evil.py");
		assert_eq!(disk_name(&evil), "evil.py");

		let dots = attachment(8, "..");
		assert_eq!(disk_name(&dots), "attachment-8");

		let plain = attachment(9, "lab1.py");
		assert_eq!(disk_name(&plain), "lab1.py");
	}

	#[test]
	fn test_an_attachment_lands_under_its_own_id() {
		// Two students' files share a name and must not share a path.
		let root = Path::new("/bundle");
		let a = attachment(1, "hw1.py");
		let b = attachment(2, "hw1.py");
		assert_eq!(attachment_path(root, &a), root.join("attachments/1/hw1.py"));
		assert_eq!(attachment_path(root, &b), root.join("attachments/2/hw1.py"));
	}

	#[test]
	fn test_a_missing_file_is_reported_not_silently_absent() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		std::fs::write(root.join(PAYLOAD_FILE), "{}").unwrap();

		let manifest: Manifest = Manifest::from([(
			"1".to_string(),
			AttachmentRecord::Stored {
				path: root.join("attachments/1/gone.py"),
				size: 10,
			},
		)]);
		std::fs::write(
			root.join(MANIFEST_FILE),
			serde_json::to_string(&manifest).unwrap(),
		)
		.unwrap();

		let (_, downloads, _) = load(root).unwrap();
		let reason = downloads
			.get(&1)
			.expect("the id is recorded")
			.as_ref()
			.expect_err("the file is gone, so this cannot be Ok");
		assert!(reason.contains("missing from the bundle"), "got {reason}");
	}

	#[test]
	fn test_a_recorded_failure_survives_the_round_trip() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		std::fs::write(root.join(PAYLOAD_FILE), "{}").unwrap();

		let manifest: Manifest = Manifest::from([(
			"5".to_string(),
			AttachmentRecord::Failed {
				error: "502 Bad Gateway".to_string(),
			},
		)]);
		std::fs::write(
			root.join(MANIFEST_FILE),
			serde_json::to_string(&manifest).unwrap(),
		)
		.unwrap();

		let (_, downloads, _) = load(root).unwrap();
		// Attempted and refused — distinct from never attempted, which is absence.
		assert_eq!(
			downloads.get(&5).unwrap().as_ref().unwrap_err(),
			"502 Bad Gateway"
		);
		assert!(!downloads.contains_key(&99));
	}

	/// The manifest deliberately records no expanded entries; they come back off the zip.
	#[test]
	fn test_a_zip_attachment_is_re_expanded_on_load() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let zip_path = root.join("attachments/3/work.zip");
		std::fs::create_dir_all(zip_path.parent().unwrap()).unwrap();

		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		use std::io::Write as _;
		zip.start_file("src/Lab5.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(b"def foo(): return 42").unwrap();
		zip.finish().unwrap();

		std::fs::write(root.join(PAYLOAD_FILE), "{}").unwrap();
		let manifest: Manifest = Manifest::from([(
			"3".to_string(),
			AttachmentRecord::Stored {
				path: zip_path.clone(),
				size: std::fs::metadata(&zip_path).unwrap().len(),
			},
		)]);
		std::fs::write(
			root.join(MANIFEST_FILE),
			serde_json::to_string(&manifest).unwrap(),
		)
		.unwrap();

		let (_, downloads, _) = load(root).unwrap();
		let entry = downloads.get(&3).unwrap().as_ref().unwrap();
		assert_eq!(entry.expanded.len(), 1);
		// The in-archive path survives, which a directory scan could not recover.
		assert_eq!(entry.expanded[0].entry, "src/Lab5.py");
		assert!(entry.expanded[0].path.is_file());
	}
}
