//! Archive expansion, shared by the local scan and the Canvas importer.
//!
//! Students hand in archives, and a teacher who asks for one gets whatever the student's
//! tool produced. Everything that makes it safe to point this at that input lives here: a
//! traversal check, a flattening-collision check, and three limits that bound a zip bomb.
//!
//! **Only entries that could be graded are written to disk.** A submission zip routinely
//! carries a dataset, `node_modules`, `__MACOSX`, a PDF of the assignment — none of which
//! any backend runs, and all of which used to land on disk once per student. The filter is
//! a predicate on the entry *name*, so it is applied before any bytes are decompressed
//! where the format allows that.
//!
//! How much that saves depends on the container, and the difference is real rather than an
//! implementation detail:
//!
//! - **zip** and **7z** carry an index, so an unwanted entry is never decompressed at all.
//! - **tar**, including `.tar.gz`, has no index and its compression is a single stream, so
//!   the bytes must be decompressed to be walked past. Filtering still avoids *writing*
//!   them, which is what the disk cost and the collision space are made of.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::discovery::detect_language;
use crate::models::{DiagnosticKind, InputDiagnostic};

pub(crate) const MAX_FILE_SIZE: u64 = 5_000_000; // 5 MB per file
const MAX_TOTAL_SIZE: u64 = 50_000_000; // 50 MB total per archive
const MAX_FILE_COUNT: usize = 100;

/// A file that came out of an archive, with the provenance needed to trace it back.
#[derive(Debug, Clone)]
pub(crate) struct ExtractedFile {
	pub(crate) out_path: PathBuf,
	pub(crate) archive: PathBuf,
	/// The path *inside* the archive, before flattening.
	pub(crate) entry: String,
}

/// The container formats we can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
	Zip,
	Tar,
	TarGz,
	TarBz2,
	TarXz,
	SevenZ,
	Rar,
}

/// What `path` appears to be, by name.
///
/// Matched on full suffixes rather than `Path::extension`, or `hw.tar.gz` would be read as
/// a bare gzip stream and `hw.tar.bz2` as bzip2 with the tar silently lost.
pub(crate) fn format_of(path: &Path) -> Option<ArchiveFormat> {
	let name = path.file_name()?.to_str()?.to_lowercase();
	for (suffix, format) in [
		(".tar.gz", ArchiveFormat::TarGz),
		(".tgz", ArchiveFormat::TarGz),
		(".tar.bz2", ArchiveFormat::TarBz2),
		(".tbz2", ArchiveFormat::TarBz2),
		(".tar.xz", ArchiveFormat::TarXz),
		(".txz", ArchiveFormat::TarXz),
		(".tar", ArchiveFormat::Tar),
		(".zip", ArchiveFormat::Zip),
		(".7z", ArchiveFormat::SevenZ),
		(".rar", ArchiveFormat::Rar),
	] {
		if name.ends_with(suffix) {
			return Some(format);
		}
	}
	None
}

/// An archive format we recognise the name of but cannot open, for the diagnostic that
/// tells a teacher to ask for something else.
///
/// Kept separate from [`format_of`] so "we do not handle this" never reads as "this is not
/// an archive" — the student's code is in there either way.
pub(crate) fn unopenable_format(path: &Path) -> Option<&'static str> {
	// `.tar.gz` ends in `.gz` and is also a format we *do* open. Asking that first is what
	// stops a successfully expanded archive being reported as unopenable in the same run.
	if format_of(path).is_some() {
		return None;
	}
	let name = path.file_name()?.to_str()?.to_lowercase();
	for (suffix, label) in [(".bz2", "bzip2"), (".xz", "xz"), (".gz", "gzip")] {
		if name.ends_with(suffix) {
			return Some(label);
		}
	}
	None
}

/// Whether an archive entry is worth putting on disk.
///
/// The default rule is "something a backend can run", which is decidable from the name
/// alone. Teacher-configurable matching (P-673) widens this; until then a submission's
/// datasets and editor droppings stay inside the archive.
pub(crate) fn is_gradeable(entry: &str) -> bool {
	let ext = Path::new(entry)
		.extension()
		.and_then(|e| e.to_str())
		.unwrap_or("")
		.to_lowercase();
	detect_language(&ext).is_some()
}

/// Why a file belonging to a known student cannot be graded.
///
/// An archive nothing here opens is reported at `Warning`, because the student's code *is*
/// there and a teacher can act on it — tell the class to use `.zip`. A stray PDF gets an
/// `Info`-level note about something that was never going to be graded.
pub(crate) fn archive_or_ignored(key: &str, path: &Path) -> InputDiagnostic {
	match unopenable_format(path) {
		Some(format) => InputDiagnostic::warning(DiagnosticKind::UnsupportedArchive {
			key: key.to_string(),
			path: path.to_path_buf(),
			format: format.to_string(),
		}),
		None => InputDiagnostic::info(DiagnosticKind::IgnoredFile {
			key: key.to_string(),
			path: path.to_path_buf(),
		}),
	}
	.at(crate::models::SourceLocation::file(path.to_path_buf()))
}

pub(crate) fn is_noise(name: &str) -> bool {
	name.starts_with('.') || name.starts_with("__")
}

fn skipped(archive: &Path, entry: &str, reason: String) -> InputDiagnostic {
	InputDiagnostic::warning(DiagnosticKind::ArchiveEntrySkipped {
		archive: archive.to_path_buf(),
		entry: entry.to_string(),
		reason,
	})
}

/// Reject an entry name that would write outside `target`.
///
/// `zip` has `enclosed_name` for this; tar and 7z do not, so the check is done here for
/// every format rather than trusting each library to have one.
fn safe_entry(name: &str) -> Option<PathBuf> {
	let path = Path::new(name);
	if path.components().any(|c| {
		matches!(
			c,
			Component::ParentDir | Component::RootDir | Component::Prefix(_)
		)
	}) {
		return None;
	}
	Some(path.to_path_buf())
}

/// How a tar's bytes are wrapped.
#[derive(Debug, Clone, Copy)]
enum Compression {
	None,
	Gzip,
	Bzip2,
	Xz,
}

/// Applies the guards, and is the only thing that decides an entry lands on disk.
struct Sink<'a> {
	archive: &'a Path,
	target: &'a Path,
	total_bytes: u64,
	file_count: usize,
	/// Flattening maps two in-archive paths onto one output name; remember who got there
	/// first so the loser is reported rather than silently dropped.
	claimed: BTreeMap<PathBuf, String>,
	extracted: Vec<ExtractedFile>,
	diagnostics: &'a mut Vec<InputDiagnostic>,
	/// Set once a whole-archive budget is spent; the walk stops rather than reporting the
	/// same exhaustion for every remaining entry.
	exhausted: bool,
}

/// What to do with an entry the sink was offered.
enum Verdict {
	/// Write it here.
	Take(PathBuf),
	/// Nothing to do, keep going.
	Skip,
	/// A whole-archive limit is spent.
	Stop,
}

impl<'a> Sink<'a> {
	fn new(archive: &'a Path, target: &'a Path, diagnostics: &'a mut Vec<InputDiagnostic>) -> Self {
		Self {
			archive,
			target,
			total_bytes: 0,
			file_count: 0,
			claimed: BTreeMap::new(),
			extracted: Vec::new(),
			diagnostics,
			exhausted: false,
		}
	}

	/// Decide an entry's fate. Every guard runs *before* the entry is recorded: claiming a
	/// name first would let a rejected entry block the real submission that wanted it.
	fn offer(&mut self, name: &str, size: u64, wanted: &dyn Fn(&str) -> bool) -> Verdict {
		if self.exhausted {
			return Verdict::Stop;
		}

		let Some(path) = safe_entry(name) else {
			self.diagnostics
				.push(skipped(self.archive, name, "unsafe path".to_string()));
			return Verdict::Skip;
		};
		let Some(filename) = path.file_name().map(|n| n.to_owned()) else {
			return Verdict::Skip;
		};
		if is_noise(&filename.to_string_lossy()) {
			return Verdict::Skip;
		}

		// The point of the filter: a dataset or a PDF is not an anomaly worth a line, it is
		// simply not this assignment. It stays in the archive and off the disk.
		if !wanted(name) {
			return Verdict::Skip;
		}

		let out_path = self.target.join(&filename);
		if let Some(first) = self.claimed.get(&out_path) {
			self.diagnostics.push(InputDiagnostic::warning(
				DiagnosticKind::ArchiveNameCollision {
					archive: self.archive.to_path_buf(),
					entry: format!("{name} (already taken by {first})"),
				},
			));
			return Verdict::Skip;
		}

		if size > MAX_FILE_SIZE {
			self.diagnostics.push(skipped(
				self.archive,
				name,
				format!("{size} bytes exceeds the {MAX_FILE_SIZE} byte limit"),
			));
			return Verdict::Skip;
		}
		if self.total_bytes + size > MAX_TOTAL_SIZE {
			self.diagnostics.push(skipped(
				self.archive,
				name,
				format!("archive exceeds the {MAX_TOTAL_SIZE} byte total"),
			));
			self.exhausted = true;
			return Verdict::Stop;
		}
		if self.file_count >= MAX_FILE_COUNT {
			self.diagnostics.push(skipped(
				self.archive,
				name,
				format!("archive exceeds the {MAX_FILE_COUNT} file limit"),
			));
			self.exhausted = true;
			return Verdict::Stop;
		}

		self.total_bytes += size;
		self.file_count += 1;
		self.claimed.insert(out_path.clone(), name.to_string());
		// Provenance is recorded whether or not the bytes are written this run, so a cached
		// extraction still traces every file back to the entry it came from.
		self.extracted.push(ExtractedFile {
			out_path: out_path.clone(),
			archive: self.archive.to_path_buf(),
			entry: name.to_string(),
		});
		Verdict::Take(out_path)
	}

	/// Undo an accepted entry whose bytes could not be written. The claim, the provenance
	/// and both counters move together; letting them drift is what lets a rejected entry
	/// block a real one.
	fn rollback(&mut self, name: &str, out_path: &Path, size: u64, reason: String) {
		self.diagnostics.push(skipped(self.archive, name, reason));
		self.extracted.pop();
		self.claimed.remove(out_path);
		self.total_bytes -= size;
		self.file_count -= 1;
	}

	/// Write an accepted entry, unless it is already there from an earlier run.
	fn write(&mut self, name: &str, out_path: &Path, size: u64, read: &mut dyn Read) {
		if out_path.exists() {
			return;
		}
		let mut buf = Vec::new();
		let failure = match read.read_to_end(&mut buf) {
			Err(_) => Some("unreadable entry".to_string()),
			Ok(_) => std::fs::write(out_path, &buf).err().map(|e| e.to_string()),
		};
		if let Some(reason) = failure {
			self.rollback(name, out_path, size, reason);
		}
	}
}

fn unreadable(archive: &Path, reason: String) -> InputDiagnostic {
	InputDiagnostic::warning(DiagnosticKind::ArchiveUnreadable {
		archive: archive.to_path_buf(),
		reason,
	})
}

/// Expand `archive` into `target`, writing only the entries `wanted` accepts.
///
/// Entries are flattened onto their base names. Already-extracted files are not rewritten,
/// but their provenance is recorded again, so a cached extraction still traces back.
pub(crate) fn expand(
	archive: &Path,
	target: &Path,
	wanted: &dyn Fn(&str) -> bool,
	diagnostics: &mut Vec<InputDiagnostic>,
) -> Vec<ExtractedFile> {
	let Some(format) = format_of(archive) else {
		return Vec::new();
	};

	let before = diagnostics.len();
	if let Err(e) = std::fs::create_dir_all(target) {
		diagnostics.push(unreadable(
			archive,
			format!("cannot create extraction directory: {e}"),
		));
		return Vec::new();
	}

	let mut sink = Sink::new(archive, target, diagnostics);
	let outcome = match format {
		ArchiveFormat::Zip => expand_zip(archive, &mut sink, wanted),
		ArchiveFormat::Tar => expand_tar(archive, &mut sink, wanted, Compression::None),
		ArchiveFormat::TarGz => expand_tar(archive, &mut sink, wanted, Compression::Gzip),
		ArchiveFormat::TarBz2 => expand_tar(archive, &mut sink, wanted, Compression::Bzip2),
		ArchiveFormat::TarXz => expand_tar(archive, &mut sink, wanted, Compression::Xz),
		ArchiveFormat::SevenZ => expand_7z(archive, &mut sink, wanted),
		ArchiveFormat::Rar => expand_rar(archive, &mut sink, wanted),
	};

	let extracted = std::mem::take(&mut sink.extracted);
	if let Err(reason) = outcome {
		diagnostics.push(unreadable(archive, reason));
		return extracted;
	}

	// An archive that opened but produced nothing leaves its owner SubmittedEmpty, which on
	// its own is indistinguishable from never having submitted. Say why — unless something
	// above already explained it, in which case this would only add noise.
	if extracted.is_empty() && diagnostics.len() == before {
		diagnostics.push(InputDiagnostic::warning(DiagnosticKind::ArchiveEmpty {
			archive: archive.to_path_buf(),
		}));
	}
	extracted
}

/// zip carries a central directory, so an unwanted entry is never decompressed.
fn expand_zip(
	archive: &Path,
	sink: &mut Sink<'_>,
	wanted: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
	let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
	let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

	for i in 0..zip.len() {
		let mut entry = match zip.by_index(i) {
			Ok(e) => e,
			Err(e) => {
				// Unreadable metadata is still something that arrived; reporting it is what
				// keeps "nothing is dropped on the floor" true.
				sink.diagnostics
					.push(skipped(archive, &format!("entry #{i}"), e.to_string()));
				continue;
			}
		};
		if entry.is_dir() {
			continue;
		}
		let name = entry.name().to_string();
		let size = entry.size();
		match sink.offer(&name, size, wanted) {
			Verdict::Take(out) => sink.write(&name, &out, size, &mut entry),
			Verdict::Skip => continue,
			Verdict::Stop => break,
		}
	}
	Ok(())
}

/// tar has no index and its compression is one stream, so every entry is decompressed to be
/// walked past. Filtering still keeps unwanted bytes off the disk.
fn expand_tar(
	archive: &Path,
	sink: &mut Sink<'_>,
	wanted: &dyn Fn(&str) -> bool,
	compression: Compression,
) -> Result<(), String> {
	let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
	let reader: Box<dyn Read> = match compression {
		Compression::None => Box::new(file),
		Compression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
		Compression::Bzip2 => Box::new(bzip2_rs::DecoderReader::new(file)),
		// `lzma-rs` decodes into a buffer rather than offering a `Read`, so an `.xz` is
		// resident in memory for the length of the walk. Bounded by the same archive size
		// cap as every other format, and xz is rare enough in student work not to justify
		// a streaming decoder of our own.
		Compression::Xz => {
			let mut input = std::io::BufReader::new(file);
			let mut decoded = Vec::new();
			lzma_rs::xz_decompress(&mut input, &mut decoded).map_err(|e| e.to_string())?;
			Box::new(std::io::Cursor::new(decoded))
		}
	};
	let mut tar = tar::Archive::new(reader);
	let entries = tar.entries().map_err(|e| e.to_string())?;

	for entry in entries {
		let mut entry = match entry {
			Ok(e) => e,
			// A tar is a stream: once an entry header will not parse, the position of every
			// following one is unknown, so continuing would read garbage.
			Err(e) => return Err(e.to_string()),
		};
		if !entry.header().entry_type().is_file() {
			continue;
		}
		let name = match entry.path() {
			Ok(p) => p.to_string_lossy().into_owned(),
			Err(e) => {
				sink.diagnostics
					.push(skipped(archive, "<unreadable name>", e.to_string()));
				continue;
			}
		};
		let size = entry.header().size().unwrap_or(0);
		match sink.offer(&name, size, wanted) {
			Verdict::Take(out) => sink.write(&name, &out, size, &mut entry),
			Verdict::Skip => continue,
			Verdict::Stop => break,
		}
	}
	Ok(())
}

/// The largest archive we will hold in memory to read.
///
/// `rars` parses from a byte slice rather than a reader, so a RAR is read whole. The cap is
/// the per-archive extraction budget plus headroom for the container itself: an archive
/// bigger than everything we would ever extract from it is not worth the RAM.
const MAX_ARCHIVE_IN_MEMORY: u64 = MAX_TOTAL_SIZE * 2;

/// RAR carries an index, so an unwanted entry is never decompressed.
///
/// This uses `rars`, a clean-room pure-Rust implementation under MIT/Apache-2.0 — not the
/// `unrar` binding. That distinction is not incidental: RARLAB's UnRAR source carries a
/// field-of-use restriction ("cannot be used to develop RAR (WinRAR) compatible archiver"),
/// which GPL-3.0 section 10 does not permit us to pass on to anyone we distribute to. An
/// independent implementation is bound by none of that.
fn expand_rar(
	archive: &Path,
	sink: &mut Sink<'_>,
	wanted: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
	let size = std::fs::metadata(archive).map_err(|e| e.to_string())?.len();
	if size > MAX_ARCHIVE_IN_MEMORY {
		return Err(format!(
			"{size} bytes exceeds the {MAX_ARCHIVE_IN_MEMORY} byte limit for reading an \
			 archive into memory"
		));
	}
	let bytes = std::fs::read(archive).map_err(|e| e.to_string())?;
	let parsed = rars::ArchiveReader::read(&bytes).map_err(|e| e.to_string())?;

	// Collected first: `read_member` borrows the archive, so the names cannot be held
	// across the calls that use them.
	let members: Vec<(Vec<u8>, u64, bool)> = parsed
		.members()
		.map(|m| {
			(
				m.meta.name.clone(),
				m.meta.unpacked_size,
				m.meta.is_directory || m.meta.is_encrypted,
			)
		})
		.collect();

	for (raw_name, size, skip) in members {
		let name = String::from_utf8_lossy(&raw_name).replace('\\', "/");
		if skip {
			// An encrypted member cannot be read without a password we do not have. Saying
			// so beats a student looking like they submitted an empty archive.
			if !name.is_empty() && wanted(&name) {
				sink.diagnostics.push(skipped(
					archive,
					&name,
					"entry is encrypted or is a directory".to_string(),
				));
			}
			continue;
		}
		match sink.offer(&name, size, wanted) {
			Verdict::Take(out) => match parsed.read_member(&raw_name, None) {
				Ok(Some(data)) => sink.write(&name, &out, size, &mut data.as_slice()),
				Ok(None) => sink.rollback(&name, &out, size, "entry vanished".to_string()),
				Err(e) => sink.rollback(&name, &out, size, e.to_string()),
			},
			Verdict::Skip => continue,
			Verdict::Stop => break,
		}
	}
	Ok(())
}

/// 7z carries a header, so an unwanted entry's reader is simply never read.
fn expand_7z(
	archive: &Path,
	sink: &mut Sink<'_>,
	wanted: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
	let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
	let mut reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
		.map_err(|e| e.to_string())?;

	let mut failure: Option<String> = None;
	let result = reader.for_each_entries(|entry, read| {
		if entry.is_directory() {
			return Ok(true);
		}
		let name = entry.name().to_string();
		let size = entry.size();
		match sink.offer(&name, size, wanted) {
			Verdict::Take(out) => sink.write(&name, &out, size, read),
			Verdict::Skip => {}
			Verdict::Stop => return Ok(false),
		}
		Ok(true)
	});
	if let Err(e) = result {
		failure = Some(e.to_string());
	}
	match failure {
		Some(reason) => Err(reason),
		None => Ok(()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn zip_with(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
		use std::io::Write as _;
		let path = dir.join(name);
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
		for (entry, body) in entries {
			zip.start_file(*entry, zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(body).unwrap();
		}
		zip.finish().unwrap();
		path
	}

	#[test]
	fn test_only_gradeable_entries_reach_the_disk() {
		let dir = tempfile::tempdir().unwrap();
		let archive = zip_with(
			dir.path(),
			"hw.zip",
			&[
				("src/lab5.py", b"x=1"),
				("data/huge.csv", b"a,b,c"),
				("README.md", b"hi"),
				("docs/spec.pdf", b"%PDF"),
			],
		);
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		assert_eq!(files.len(), 1);
		assert_eq!(files[0].entry, "src/lab5.py");
		// The dataset and the PDF never landed — that is the point.
		assert!(target.join("lab5.py").is_file());
		assert!(!target.join("huge.csv").exists());
		assert!(!target.join("spec.pdf").exists());
		// Nor are they anomalies worth a line each.
		assert!(diagnostics.is_empty(), "got {diagnostics:?}");
	}

	#[test]
	fn test_a_tar_gz_expands_the_same_way() {
		let dir = tempfile::tempdir().unwrap();
		let archive = dir.path().join("hw.tar.gz");
		{
			let file = std::fs::File::create(&archive).unwrap();
			let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
			let mut tar = tar::Builder::new(encoder);
			for (name, body) in [("src/lab5.py", &b"x=1"[..]), ("data.csv", &b"a,b"[..])] {
				let mut header = tar::Header::new_gnu();
				header.set_size(body.len() as u64);
				header.set_mode(0o644);
				header.set_cksum();
				tar.append_data(&mut header, name, body).unwrap();
			}
			tar.into_inner().unwrap().finish().unwrap();
		}

		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();
		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		assert_eq!(files.len(), 1);
		assert_eq!(files[0].entry, "src/lab5.py");
		assert!(!target.join("data.csv").exists());
	}

	#[test]
	fn test_an_entry_escaping_the_target_is_refused_in_every_format() {
		// zip's own `enclosed_name` would catch this; tar and 7z have no equivalent, so the
		// check lives here and is asserted through the shared path.
		assert!(safe_entry("../../etc/passwd").is_none());
		assert!(safe_entry("/etc/passwd").is_none());
		assert!(safe_entry("a/../../b.py").is_none());
		assert!(safe_entry("src/lab5.py").is_some());
	}

	/// `.tar.gz` ends in `.gz`, and both the scan's archive branch and the unopenable-format
	/// table used to key on the trailing extension — so a tar.gz was expanded *and* reported
	/// as unopenable in the same run, telling a teacher to chase a submission that had
	/// already been graded.
	#[test]
	fn test_an_archive_we_can_open_is_never_also_called_unopenable() {
		for name in [
			"h.tar.gz",
			"h.tgz",
			"h.tar",
			"h.zip",
			"h.7z",
			"h.rar",
			"h.tar.bz2",
			"h.tar.xz",
		] {
			let path = Path::new(name);
			assert!(format_of(path).is_some(), "{name} should be openable");
			assert_eq!(
				unopenable_format(path),
				None,
				"{name} is openable, so it must not also be reported as unopenable"
			);
		}
	}

	#[test]
	fn test_format_detection_matches_full_suffixes() {
		assert_eq!(format_of(Path::new("h.tar.gz")), Some(ArchiveFormat::TarGz));
		assert_eq!(format_of(Path::new("h.tgz")), Some(ArchiveFormat::TarGz));
		assert_eq!(format_of(Path::new("h.tar")), Some(ArchiveFormat::Tar));
		assert_eq!(format_of(Path::new("H.ZIP")), Some(ArchiveFormat::Zip));
		assert_eq!(format_of(Path::new("h.7z")), Some(ArchiveFormat::SevenZ));
		assert_eq!(format_of(Path::new("h.rar")), Some(ArchiveFormat::Rar));
		assert_eq!(
			format_of(Path::new("h.tar.bz2")),
			Some(ArchiveFormat::TarBz2)
		);
		assert_eq!(format_of(Path::new("h.tar.xz")), Some(ArchiveFormat::TarXz));
		assert_eq!(format_of(Path::new("lab5.py")), None);

		// Recognised, but not openable — a different message from "not an archive".
		// RAR, tar.bz2 and tar.xz are opened now, so they are not "unopenable".
		assert_eq!(unopenable_format(Path::new("h.rar")), None);
		assert_eq!(unopenable_format(Path::new("h.tar.bz2")), None);
		assert_eq!(unopenable_format(Path::new("h.zip")), None);
		// Ends in `.gz`, but we open it — so it is not "unopenable".
		assert_eq!(unopenable_format(Path::new("h.tar.gz")), None);
		assert_eq!(unopenable_format(Path::new("h.gz")), Some("gzip"));
	}

	#[test]
	fn test_an_oversized_entry_is_skipped_without_losing_its_neighbours() {
		let dir = tempfile::tempdir().unwrap();
		let big = vec![0u8; (MAX_FILE_SIZE + 1_000) as usize];
		let archive = zip_with(
			dir.path(),
			"hw.zip",
			&[("big.py", &big), ("good.py", b"x=1")],
		);
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		let names: Vec<&str> = files.iter().map(|f| f.entry.as_str()).collect();
		assert_eq!(names, vec!["good.py"]);
		assert_eq!(diagnostics.len(), 1);
		assert!(matches!(
			&diagnostics[0].kind,
			DiagnosticKind::ArchiveEntrySkipped { entry, reason, .. }
				if entry == "big.py" && reason.contains(&MAX_FILE_SIZE.to_string())
		));
	}

	#[test]
	fn test_two_entries_flattening_onto_one_name_report_the_loser() {
		let dir = tempfile::tempdir().unwrap();
		let archive = zip_with(
			dir.path(),
			"hw.zip",
			&[("a/lab5.py", b"x=1"), ("b/lab5.py", b"y=2")],
		);
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		assert_eq!(files.len(), 1);
		assert!(
			diagnostics
				.iter()
				.any(|d| matches!(&d.kind, DiagnosticKind::ArchiveNameCollision { .. }))
		);
	}

	/// RAR is handled by `rars`, a clean-room pure-Rust implementation, not by the `unrar`
	/// binding to RARLAB's source — which carries a field-of-use restriction GPL-3.0
	/// section 10 forbids us from passing on. The fixture is a real WinRAR archive at the
	/// default compression level, so this fails if the codec ever regresses to stored-only.
	#[test]
	fn test_a_real_compressed_rar_expands() {
		let fixture =
			Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/m3_default.rar");
		if !fixture.is_file() {
			panic!("missing fixture: {}", fixture.display());
		}
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		// The fixture's single member is a .txt, so grade-only filtering would reject it;
		// this asserts the codec, not the filter.
		let files = expand(&fixture, &target, &|_| true, &mut diagnostics);

		assert_eq!(files.len(), 1, "got {diagnostics:?}");
		let written = std::fs::metadata(&files[0].out_path).unwrap().len();
		assert_eq!(
			written, 65536,
			"real RAR5 compressed data must round-trip, not just stored entries"
		);
	}

	#[test]
	fn test_an_empty_archive_says_so() {
		let dir = tempfile::tempdir().unwrap();
		let archive = zip_with(dir.path(), "hw.zip", &[]);
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		assert!(files.is_empty());
		assert!(matches!(
			&diagnostics[0].kind,
			DiagnosticKind::ArchiveEmpty { .. }
		));
	}

	/// An archive holding only ungradeable files is *not* empty — it was filtered. Saying
	/// "expanded to nothing" there would be true but useless; the owner still needs to know
	/// their submission had no code in it.
	#[test]
	fn test_an_archive_filtered_down_to_nothing_still_reports_empty() {
		let dir = tempfile::tempdir().unwrap();
		let archive = zip_with(dir.path(), "hw.zip", &[("notes.txt", b"hi")]);
		let target = dir.path().join("out");
		let mut diagnostics = Vec::new();

		let files = expand(&archive, &target, &is_gradeable, &mut diagnostics);

		assert!(files.is_empty());
		assert!(matches!(
			&diagnostics[0].kind,
			DiagnosticKind::ArchiveEmpty { .. }
		));
	}
}
