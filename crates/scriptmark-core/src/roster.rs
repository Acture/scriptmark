use std::collections::HashMap;
use std::path::Path;

/// Load a roster CSV mapping student IDs to names.
///
/// Expected format: `name,_,student_id` (header row skipped).
/// Handles UTF-8 BOM.
pub fn load_roster(path: &Path) -> Result<HashMap<String, String>, RosterError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| RosterError::IoError(path.to_path_buf(), e))?;

    // Strip UTF-8 BOM if present
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let mut roster = HashMap::new();

    for result in reader.records() {
        let record = result.map_err(|e| RosterError::CsvError(path.to_path_buf(), e))?;

        // Format: name, _, student_id (or name, student_id)
        let name = record.get(0).unwrap_or("").trim().to_string();
        let student_id = if record.len() >= 3 {
            record.get(2).unwrap_or("").trim().to_string()
        } else if record.len() >= 2 {
            record.get(1).unwrap_or("").trim().to_string()
        } else {
            continue;
        };

        if !student_id.is_empty() {
            roster.insert(student_id, name);
        }
    }

    Ok(roster)
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

    #[test]
    fn test_load_roster() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roster.csv");
        std::fs::write(
            &path,
            "name,class,student_id\nAlice,A,alice123\nBob,B,bob456\n",
        )
        .unwrap();

        let roster = load_roster(&path).unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster["alice123"], "Alice");
        assert_eq!(roster["bob456"], "Bob");
    }

    #[test]
    fn test_load_roster_with_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roster.csv");
        std::fs::write(&path, "\u{feff}name,class,student_id\nAlice,A,alice123\n").unwrap();

        let roster = load_roster(&path).unwrap();
        assert_eq!(roster["alice123"], "Alice");
    }
}
