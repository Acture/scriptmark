use serde::{Deserialize, Serialize};

/// Grade curve method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CurveMethod {
    #[default]
    None,
    Linear,
    Sqrt,
}

/// Configuration for grade curving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurveConfig {
    #[serde(default)]
    pub method: CurveMethod,
    #[serde(default = "default_lower")]
    pub lower_bound: f64,
    #[serde(default = "default_upper")]
    pub upper_bound: f64,
}

fn default_lower() -> f64 {
    60.0
}
fn default_upper() -> f64 {
    100.0
}

impl Default for CurveConfig {
    fn default() -> Self {
        Self {
            method: CurveMethod::None,
            lower_bound: 60.0,
            upper_bound: 100.0,
        }
    }
}

/// Course-level configuration (from course.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseConfig {
    pub course: CourseInfo,
    #[serde(default)]
    pub grading: CurveConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseInfo {
    pub name: String,
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "python".to_string()
}

/// Assignment-level configuration (from assignment.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentConfig {
    pub assignment: AssignmentInfo,
    /// Expected student files.
    #[serde(default)]
    pub files: Vec<FilePattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentInfo {
    pub name: String,
    #[serde(default = "default_tests_dir")]
    pub tests_dir: String,
}

fn default_tests_dir() -> String {
    "tests".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePattern {
    pub pattern: String,
}
