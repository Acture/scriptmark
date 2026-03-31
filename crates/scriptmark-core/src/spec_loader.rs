use std::path::Path;

use crate::models::{AssignmentConfig, CourseConfig, TestSpec};

/// Load a test specification from a TOML file.
pub fn load_spec(path: &Path) -> Result<TestSpec, SpecError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
    let spec: TestSpec =
        toml::from_str(&content).map_err(|e| SpecError::ParseError(path.to_path_buf(), e))?;
    Ok(spec)
}

/// Load all test specifications from a directory (*.toml files).
pub fn load_specs_from_dir(dir: &Path) -> Result<Vec<TestSpec>, SpecError> {
    if !dir.is_dir() {
        return Err(SpecError::NotADirectory(dir.to_path_buf()));
    }

    let mut specs = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| SpecError::IoError(dir.to_path_buf(), e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
        .collect();

    // Deterministic ordering
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let spec = load_spec(&entry.path())?;
        specs.push(spec);
    }

    Ok(specs)
}

/// Load course configuration from course.toml.
pub fn load_course_config(path: &Path) -> Result<CourseConfig, SpecError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
    let config: CourseConfig =
        toml::from_str(&content).map_err(|e| SpecError::ParseError(path.to_path_buf(), e))?;
    Ok(config)
}

/// Load assignment configuration from assignment.toml.
pub fn load_assignment_config(path: &Path) -> Result<AssignmentConfig, SpecError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
    let config: AssignmentConfig =
        toml::from_str(&content).map_err(|e| SpecError::ParseError(path.to_path_buf(), e))?;
    Ok(config)
}

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("not a directory: {0}")]
    NotADirectory(std::path::PathBuf),
    #[error("IO error reading {0}: {1}")]
    IoError(std::path::PathBuf, std::io::Error),
    #[error("TOML parse error in {0}: {1}")]
    ParseError(std::path::PathBuf, toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_spec() {
        let dir = tempfile::tempdir().unwrap();
        let spec_path = dir.path().join("test_larger.toml");
        std::fs::write(
            &spec_path,
            r#"
[meta]
name = "find_larger_number"
file = "Lab5_1.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "3 < 5"
args = [3, 5]
expect = 5

[[cases]]
name = "negative"
args = [-3, -2]
expect = -2

[[cases]]
name = "raises TypeError"
args = ["a", 1]
expect_error = "TypeError"
"#,
        )
        .unwrap();

        let spec = load_spec(&spec_path).unwrap();
        assert_eq!(spec.meta.name, "find_larger_number");
        assert_eq!(spec.meta.language, "python");
        assert_eq!(spec.meta.function.as_deref(), Some("find_larger_number"));
        assert_eq!(spec.cases.len(), 3);
        assert_eq!(spec.cases[0].name, "3 < 5");
        assert_eq!(spec.cases[2].expect_error.as_deref(), Some("TypeError"));
    }

    #[test]
    fn test_load_course_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("course.toml");
        std::fs::write(
            &config_path,
            r#"
[course]
name = "GEEC Python"
language = "python"

[grading]
method = "sqrt"
lower_bound = 60
upper_bound = 100
"#,
        )
        .unwrap();

        let config = load_course_config(&config_path).unwrap();
        assert_eq!(config.course.name, "GEEC Python");
        assert_eq!(config.course.language, "python");
    }
}
