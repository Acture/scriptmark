use serde::{Deserialize, Serialize};

/// Grading policy — how to convert pass rate to final grade.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GradingPolicy {
	/// Named template: "none", "linear", "sqrt", "log", "strict"
	Template(TemplatePolicy),
	/// Custom Rhai formula
	Formula(FormulaPolicy),
}

/// Named grading template with configurable bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplatePolicy {
	/// Template name
	pub template: String,
	/// Lower bound (default 60)
	#[serde(default = "default_lower")]
	pub lower: f64,
	/// Upper bound (default 100)
	#[serde(default = "default_upper")]
	pub upper: f64,
}

/// Custom grading formula evaluated via Rhai.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaPolicy {
	/// Rhai expression. Variables: rate, passed, total, lint_score
	pub formula: String,
}

impl Default for GradingPolicy {
	fn default() -> Self {
		Self::Template(TemplatePolicy {
			template: "sqrt".to_string(),
			lower: 60.0,
			upper: 100.0,
		})
	}
}

fn default_lower() -> f64 {
	60.0
}
fn default_upper() -> f64 {
	100.0
}

/// Course-level configuration (from course.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseConfig {
	pub course: CourseInfo,
	#[serde(default)]
	pub grading: GradingPolicy,
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
