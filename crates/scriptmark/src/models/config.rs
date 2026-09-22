use serde::{Deserialize, Serialize};

use crate::models::GradingItem;

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

/// Which submission attempt to grade when a source reports several.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPolicy {
	/// Highest attempt number wins.
	#[default]
	Latest,
	/// Lowest attempt number wins.
	Earliest,
}

/// Assignment-level configuration (from assignment.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentConfig {
	pub assignment: AssignmentInfo,
	/// Expected student files.
	#[serde(default)]
	pub files: Vec<FilePattern>,
	/// The items this assignment is marked on. Each `id` is a test spec's `[meta] name`.
	/// Left empty, the items are derived from the specs that were loaded.
	#[serde(default)]
	pub items: Vec<GradingItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentInfo {
	pub name: String,
	#[serde(default = "default_tests_dir")]
	pub tests_dir: String,
	/// Canvas course id. Kept apart from the assignment id and from any student identity.
	#[serde(default)]
	pub canvas_course_id: Option<u64>,
	/// Canvas assignment id.
	#[serde(default)]
	pub canvas_assignment_id: Option<u64>,
	/// Which attempt to grade when the source reports several.
	#[serde(default)]
	pub attempt_policy: AttemptPolicy,
}

fn default_tests_dir() -> String {
	"tests".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePattern {
	pub pattern: String,
}
