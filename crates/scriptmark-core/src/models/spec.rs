use serde::{Deserialize, Serialize};

/// Configuration for lint-based code style scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintConfig {
    /// Lint command template. `{file}` is replaced with student file path.
    pub command: String,
    /// Max warnings that maps to 0% style score.
    #[serde(default = "default_max_warnings")]
    pub max_warnings: usize,
    /// Weight in final grade (0.0-1.0). 0.1 = 10% of grade.
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_max_warnings() -> usize {
    10
}

fn default_weight() -> f64 {
    0.1
}

/// How to check the result of a test case.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CheckMethod {
    /// Shorthand: just a string like "sorted", "exact", "contains"
    Builtin(String),
    /// Detailed spec with parameters
    Detailed(CheckSpec),
}

impl Default for CheckMethod {
    fn default() -> Self {
        Self::Builtin("exact".to_string())
    }
}

/// Detailed checker specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckSpec {
    /// Built-in checker name
    #[serde(default)]
    pub builtin: Option<String>,
    /// Rhai inline expression
    #[serde(default)]
    pub rhai: Option<String>,
    /// Path to Python verifier script
    #[serde(default)]
    pub python: Option<String>,
    /// Path to executable verifier
    #[serde(default)]
    pub exec: Option<String>,
    /// Path to WASM verifier module
    #[serde(default)]
    pub wasm: Option<String>,
    /// Tolerance for approx checker
    #[serde(default)]
    pub tolerance: Option<f64>,
}

/// Oracle — how to determine the expected output for generated inputs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Oracle {
    /// Teacher's reference implementation file. Same function name, compare outputs.
    #[serde(default)]
    pub reference: Option<String>,
    /// Rhai expression computing expected value from generated args.
    #[serde(default)]
    pub rhai: Option<String>,
    /// Built-in checker name (just verifies a property, no expected value).
    #[serde(default)]
    pub check: Option<String>,
    /// Python script oracle.
    #[serde(default)]
    pub python: Option<String>,
}

/// Parametrize configuration — auto-generate test cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parametrize {
    /// Number of test cases to generate.
    pub count: usize,
    /// Random seed for reproducibility.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Generator expressions per argument. Key = arg name, Value = generator string.
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
    /// How to determine the expected output.
    #[serde(default)]
    pub oracle: Oracle,
}

/// A single test case within a test spec.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestCase {
    pub name: String,

    /// Optional ID for this case (allows other cases/fixtures to reference its result).
    #[serde(default)]
    pub id: Option<String>,

    /// Arguments to pass to the function (for function-call tests).
    /// May contain `$ref` strings referencing fixture results.
    #[serde(default)]
    pub args: Vec<serde_json::Value>,

    /// Expected return value (for exact/approx matching).
    #[serde(default)]
    pub expect: Option<serde_json::Value>,

    /// Expected error type (e.g. "TypeError").
    #[serde(default)]
    pub expect_error: Option<String>,

    /// Stdin input (for IO-based tests).
    #[serde(default)]
    pub stdin: Option<String>,

    /// Expected stdout (for IO-based tests).
    #[serde(default)]
    pub expected_stdout: Option<String>,

    /// How to check the result. Defaults to exact match.
    #[serde(default)]
    pub check: Option<CheckMethod>,

    /// Per-case timeout override in seconds.
    #[serde(default)]
    pub timeout: Option<u64>,

    /// Parametrize configuration for auto-generating test cases.
    #[serde(default)]
    pub parametrize: Option<Parametrize>,
}

/// Metadata for a test spec file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMeta {
    pub name: String,

    /// Student file to test (suffix match, e.g. "Lab5_1.py").
    pub file: String,

    /// Language of the student code.
    pub language: String,

    /// Function name to call (for function-call tests).
    #[serde(default)]
    pub function: Option<String>,

    /// Compile command template (for compiled languages).
    /// Placeholders: {source}, {output}
    #[serde(default)]
    pub compile: Option<String>,
}

/// A setup step — calls a function, stores result for `$ref`. Not scored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStep {
    /// Required. Used as `$id` in args references.
    pub id: String,

    /// Call a student function and store the result.
    #[serde(default)]
    pub function: Option<String>,

    /// Arguments for the function call. May contain `$ref` strings.
    #[serde(default)]
    pub args: Vec<serde_json::Value>,

    /// Run a teacher script and use its stdout (JSON) as the value.
    #[serde(default)]
    pub file: Option<String>,
}

/// A complete test specification (one TOML file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSpec {
    pub meta: TestMeta,
    /// Static variables — injected as Python globals + available as `$ref`.
    #[serde(default)]
    pub vars: std::collections::HashMap<String, serde_json::Value>,
    /// Setup steps — call functions, store results. Not scored.
    #[serde(default)]
    pub setup: Vec<SetupStep>,
    /// Test cases — scored.
    pub cases: Vec<TestCase>,
    /// Optional lint-based style scoring.
    #[serde(default)]
    pub lint: Option<LintConfig>,
}
