use serde::{Deserialize, Serialize};

use crate::models::{Aggregation, GradingItem};

/// How a grade comes from an item's cases, and from the items a total.
///
/// Read from `assignment.toml [grading]`; every field has a default, and omitting the
/// table, leaving it empty and `GradingConfig::default()` mean the same policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GradingConfig {
	/// A roster student who submitted nothing, or only empty files.
	pub missing: MissingPolicy,
	/// An item whose file the student did not hand in.
	pub missing_file: MissingPolicy,
	/// What a perfect score is worth: the total is `score / max * scale`.
	pub scale: f64,
	/// Decimal places the grades are rounded to, half away from zero.
	pub decimals: u8,
	pub curve: Curve,
	/// Points the lint score is worth, on top of the items. Lint counts for nothing
	/// unless this is declared.
	pub lint_points: Option<u32>,
}

impl Default for GradingConfig {
	fn default() -> Self {
		Self {
			missing: MissingPolicy::Withheld,
			missing_file: MissingPolicy::Withheld,
			scale: 100.0,
			decimals: 2,
			curve: Curve::Raw,
			lint_points: None,
		}
	}
}

/// What happens to work that is not there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingPolicy {
	/// No grade, and nothing is pushed.
	#[default]
	Withheld,
	/// A real 0, recorded with why.
	Zero,
}

/// How the raw fraction becomes the grade. Only ever what the teacher declared.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Curve {
	/// `fraction * scale`.
	#[default]
	Raw,
	/// A named curve mapping the fraction onto `lower..=upper`.
	Template {
		name: CurveTemplate,
		lower: f64,
		upper: f64,
	},
	/// A Rhai expression over `score`, `max`, `fraction` and `scale`.
	Formula { formula: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveTemplate {
	Linear,
	Sqrt,
	Log,
	Strict,
}

/// One `[[items]]` entry as the teacher writes it. Stricter than [`GradingItem`]: how an
/// item's cases add up must be said, not assumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDecl {
	pub id: String,
	#[serde(default)]
	pub title: Option<String>,
	#[serde(default = "one_point")]
	pub points: u32,
	pub aggregation: Aggregation,
}

fn one_point() -> u32 {
	1
}

impl From<ItemDecl> for GradingItem {
	fn from(decl: ItemDecl) -> Self {
		Self {
			id: decl.id,
			title: decl.title,
			points: decl.points,
			aggregation: decl.aggregation,
		}
	}
}

/// Course-level configuration (from course.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CourseConfig {
	pub course: CourseInfo,
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
#[serde(deny_unknown_fields)]
pub struct AssignmentConfig {
	pub assignment: AssignmentInfo,
	/// Expected student files.
	#[serde(default)]
	pub files: Vec<FilePattern>,
	/// The items this assignment is marked on. Each `id` is a test spec's `[meta] name`.
	/// Left empty, the items are derived from the specs that were loaded.
	#[serde(default)]
	pub items: Vec<ItemDecl>,
	#[serde(default)]
	pub grading: GradingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
