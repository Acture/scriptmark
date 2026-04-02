use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::models::{StudentFile, SubmissionSet};

/// Map file extensions to language identifiers.
fn detect_language(ext: &str) -> Option<&'static str> {
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

/// Extract student ID from a filename.
///
/// Convention: `{student_id}_{rest}.ext` (e.g. `alice_Lab5_1.py` → `alice`)
fn extract_sid(filename: &str) -> Option<String> {
	let stem = Path::new(filename).file_stem()?.to_str()?;
	let sid = stem.split('_').next()?;
	if sid.is_empty() {
		return None;
	}
	Some(sid.to_string())
}

/// Extract .zip archives in a directory to `.scriptmark_extracted/{archive_stem}/`.
///
/// Returns list of directories created. Skips archives that have already been extracted
/// (directory exists and is non-empty). Silently skips corrupt/unreadable archives.
fn extract_archives(dir: &Path) -> Vec<PathBuf> {
	let extract_root = dir.join(".scriptmark_extracted");
	let mut created = Vec::new();

	let entries = match std::fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return created,
	};

	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_file() {
			continue;
		}
		let ext = path
			.extension()
			.and_then(|e| e.to_str())
			.unwrap_or("")
			.to_lowercase();
		if ext != "zip" {
			continue;
		}

		let stem = path
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or("unknown");
		let target = extract_root.join(stem);

		// Skip if already extracted
		if target.is_dir()
			&& std::fs::read_dir(&target)
				.map(|mut d| d.next().is_some())
				.unwrap_or(false)
		{
			created.push(target);
			continue;
		}

		// Extract
		let file = match std::fs::File::open(&path) {
			Ok(f) => f,
			Err(_) => continue,
		};
		let mut archive = match zip::ZipArchive::new(file) {
			Ok(a) => a,
			Err(e) => {
				eprintln!("[WARN] Skipping corrupt archive {}: {e}", path.display());
				continue;
			}
		};

		if let Err(e) = std::fs::create_dir_all(&target) {
			eprintln!("[WARN] Cannot create extract dir {}: {e}", target.display());
			continue;
		}

		const MAX_FILE_SIZE: u64 = 5_000_000; // 5 MB per file
		const MAX_TOTAL_SIZE: u64 = 50_000_000; // 50 MB total per archive
		const MAX_FILE_COUNT: usize = 100;

		let mut total_bytes: u64 = 0;
		let mut file_count: usize = 0;

		for i in 0..archive.len() {
			let mut entry = match archive.by_index(i) {
				Ok(e) => e,
				Err(_) => continue,
			};

			if entry.is_dir() {
				continue;
			}

			// Check uncompressed size before reading
			if entry.size() > MAX_FILE_SIZE {
				eprintln!(
					"[WARN] Skipping oversized entry in {}: {} ({} bytes)",
					path.display(),
					entry.name(),
					entry.size()
				);
				continue;
			}

			if total_bytes + entry.size() > MAX_TOTAL_SIZE {
				eprintln!(
					"[WARN] Archive {} exceeds total extraction limit ({}B), stopping",
					path.display(),
					MAX_TOTAL_SIZE
				);
				break;
			}

			if file_count >= MAX_FILE_COUNT {
				eprintln!(
					"[WARN] Archive {} exceeds file count limit ({}), stopping",
					path.display(),
					MAX_FILE_COUNT
				);
				break;
			}

			let name = match entry.enclosed_name() {
				Some(n) => n.to_owned(),
				None => continue, // skip path traversal attempts
			};

			// Flatten: extract to target/{filename} regardless of subdirectories in archive
			let filename = match name.file_name() {
				Some(n) => n.to_owned(),
				None => continue,
			};

			// Skip __pycache__, .DS_Store, etc.
			let fname_str = filename.to_string_lossy();
			if fname_str.starts_with('.') || fname_str.starts_with("__") {
				continue;
			}

			let out_path = target.join(&filename);
			if out_path.exists() {
				continue; // don't overwrite
			}

			let mut buf = Vec::new();
			if entry.read_to_end(&mut buf).is_ok() {
				let _ = std::fs::write(&out_path, &buf);
				total_bytes += buf.len() as u64;
				file_count += 1;
			}
		}

		created.push(target);
	}

	created
}

/// Scan directories for student submission files and group by student ID.
///
/// Only includes files with recognized language extensions.
/// If `extensions` is provided, only includes files matching those extensions.
pub fn discover_submissions(
	paths: &[impl AsRef<Path>],
	extensions: Option<&[&str]>,
) -> Result<SubmissionSet, DiscoveryError> {
	let mut by_student: HashMap<String, Vec<StudentFile>> = HashMap::new();

	// Phase 0: Extract archives in each submission directory
	let mut extra_dirs: Vec<PathBuf> = Vec::new();
	for dir_path in paths {
		let extracted = extract_archives(dir_path.as_ref());
		extra_dirs.extend(extracted);
	}

	// Combine original paths + extracted subdirectories
	let all_paths: Vec<&Path> = paths
		.iter()
		.map(|p| p.as_ref())
		.chain(extra_dirs.iter().map(|p| p.as_path()))
		.collect();

	for dir_path in &all_paths {
		if !dir_path.is_dir() {
			return Err(DiscoveryError::NotADirectory(dir_path.to_path_buf()));
		}

		let entries = std::fs::read_dir(dir_path)
			.map_err(|e| DiscoveryError::IoError(dir_path.to_path_buf(), e))?;

		for entry in entries {
			let entry = entry.map_err(|e| DiscoveryError::IoError(dir_path.to_path_buf(), e))?;
			let path = entry.path();

			if !path.is_file() {
				continue;
			}

			let ext = match path.extension().and_then(|e| e.to_str()) {
				Some(e) => e,
				None => continue,
			};

			// Filter by allowed extensions if specified
			if let Some(allowed) = extensions
				&& !allowed.contains(&ext)
			{
				continue;
			}

			let language = match detect_language(ext) {
				Some(lang) => lang.to_string(),
				None => continue,
			};

			let filename = match path.file_name().and_then(|n| n.to_str()) {
				Some(n) => n,
				None => continue,
			};

			// For files inside .scriptmark_extracted/, prefer SID from the parent dir
			// (which is the archive stem, e.g. "bob_12345_67890_Lab5")
			let is_extracted = dir_path
				.components()
				.any(|c| c.as_os_str() == ".scriptmark_extracted");

			let sid = if is_extracted {
				// Try parent dir first (archive stem has SID), then file
				dir_path
					.file_name()
					.and_then(|n| n.to_str())
					.and_then(extract_sid)
					.or_else(|| extract_sid(filename))
			} else {
				extract_sid(filename)
			};

			let sid = match sid {
				Some(sid) => sid,
				None => continue,
			};

			by_student
				.entry(sid)
				.or_default()
				.push(StudentFile { path, language });
		}
	}

	// Sort files within each student for deterministic ordering
	for files in by_student.values_mut() {
		files.sort_by(|a, b| a.path.cmp(&b.path));
	}

	Ok(SubmissionSet { by_student })
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
	fn test_discover_extracts_zip_archives() {
		let dir = tempfile::tempdir().unwrap();

		// Normal .py file
		std::fs::write(dir.path().join("alice_Lab5.py"), "pass").unwrap();

		// Create a .zip containing a .py file
		let zip_path = dir.path().join("bob_12345_67890_Lab5.zip");
		let file = std::fs::File::create(&zip_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		zip.start_file("Lab5.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		use std::io::Write;
		zip.write_all(b"def foo(): return 42").unwrap();
		zip.finish().unwrap();

		let result = discover_submissions(&[dir.path()], None).unwrap();

		// alice from .py, bob from extracted .zip
		assert_eq!(result.student_count(), 2);
		assert!(result.by_student.contains_key("alice"));
		assert!(result.by_student.contains_key("bob"));
	}

	#[test]
	fn test_discover_submissions() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("alice_Lab5.py"), "pass").unwrap();
		std::fs::write(dir.path().join("bob_Lab5.py"), "pass").unwrap();
		std::fs::write(dir.path().join("alice_Lab6.py"), "pass").unwrap();
		std::fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();

		let result = discover_submissions(&[dir.path()], None).unwrap();
		assert_eq!(result.student_count(), 2);
		assert_eq!(result.by_student["alice"].len(), 2);
		assert_eq!(result.by_student["bob"].len(), 1);
		assert_eq!(result.languages(), vec!["python"]);
	}
}
