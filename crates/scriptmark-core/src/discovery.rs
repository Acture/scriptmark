use std::collections::HashMap;
use std::path::Path;

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

/// Scan directories for student submission files and group by student ID.
///
/// Only includes files with recognized language extensions.
/// If `extensions` is provided, only includes files matching those extensions.
pub fn discover_submissions(
    paths: &[impl AsRef<Path>],
    extensions: Option<&[&str]>,
) -> Result<SubmissionSet, DiscoveryError> {
    let mut by_student: HashMap<String, Vec<StudentFile>> = HashMap::new();

    for dir_path in paths {
        let dir_path = dir_path.as_ref();
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

            let sid = match extract_sid(filename) {
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
