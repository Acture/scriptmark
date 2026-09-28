//! The execution interface: what the orchestrator hands a language backend, and what it
//! gets back. Nothing here judges; `runner::judge` turns observations into results.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{StudentFile, Target, TestSpec};

/// A language backend.
pub trait Executor: Send + Sync + 'static {
	/// Language identifier (e.g. "python").
	fn language(&self) -> &str;

	/// Pick the student's file for a spec. P-673 owns the rule.
	fn locate<'a>(&self, files: &'a [StudentFile], spec: &TestSpec) -> Option<&'a StudentFile>;

	/// Import the spec's teacher modules the way a unit would, and report their exports.
	fn inspect(
		&self,
		spec: &TestSpec,
		timeout_secs: u64,
	) -> impl Future<Output = Result<TeacherRuntime, String>> + Send;

	/// Run one unit — an independent case, or a scenario — and report what happened.
	fn run(&self, plan: &UnitPlan) -> impl Future<Output = UnitObservation> + Send;
}

/// Whose code the unit's subject file is. Decides lookup, and who owns its failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
	Student,
	/// A teacher's reference implementation: looked up exactly, never fuzzily.
	Reference,
}

/// Everything a backend needs to run one unit.
#[derive(Debug, Clone)]
pub struct UnitPlan {
	pub subject: Subject,
	/// Copied into the unit's working directory before running.
	pub file: PathBuf,
	/// Run the file as `__main__` with this stdin, instead of calling functions.
	pub script: Option<ScriptRun>,
	pub imports: Vec<String>,
	pub vars: Arc<BTreeMap<String, Value>>,
	/// `(source, relative destination)` pairs staged into the working directory.
	pub data_files: Vec<(PathBuf, String)>,
	pub allowed_imports: Vec<String>,
	pub load_timeout: u64,
	pub setup: Vec<CallPlan>,
	pub steps: Vec<CallPlan>,
}

impl UnitPlan {
	/// Seconds the unit may run in total: every call's own timeout, plus slack. The
	/// kernel's CPU limit sits just above it as a backstop for a single busy core.
	pub fn deadline_secs(&self) -> u64 {
		let calls = match &self.script {
			Some(script) => script.timeout,
			None => self
				.setup
				.iter()
				.chain(&self.steps)
				.map(CallPlan::budget)
				.fold(self.load_timeout, u64::saturating_add),
		};
		calls.saturating_add(2)
	}
}

/// A script run: the unit's only call.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptRun {
	pub stdin: Option<String>,
	pub timeout: u64,
	pub files: Vec<String>,
}

/// One call in a unit.
#[derive(Debug, Clone, Serialize)]
pub struct CallPlan {
	pub target: Target,
	pub args: Vec<Value>,
	pub stdin: Option<String>,
	pub timeout: u64,
	/// Store the value under this name for later calls.
	pub id: Option<String>,
	/// Files to read from the working directory after the call.
	pub files: Vec<String>,
	/// An in-process checker to run on the live value.
	pub check: Option<InProcessCheck>,
}

impl CallPlan {
	fn budget(&self) -> u64 {
		self.timeout
			.saturating_add(self.check.as_ref().map_or(0, |_| self.timeout))
	}
}

/// A teacher function judging a live value: `fn(result, expected, **names_in_scope)`.
#[derive(Debug, Clone, Serialize)]
pub struct InProcessCheck {
	pub function: String,
	pub expected: Option<Value>,
}

/// What the teacher modules export, as found by importing them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeacherRuntime {
	pub exports: BTreeMap<String, Export>,
	/// Names two modules export as different objects, with both modules' paths.
	#[serde(default)]
	pub duplicates: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
	pub callable: bool,
	/// The parameters, when the export is callable and introspectable.
	#[serde(default)]
	pub params: Option<Vec<Param>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
	pub name: String,
	/// Python's `inspect.Parameter.kind`, lowercased (`positional_or_keyword`, `var_keyword`, …).
	pub kind: String,
	/// Whether the parameter has a default.
	pub default: bool,
}

impl Param {
	/// `*args` or `**kwargs`: never filled by name.
	pub fn is_variadic(&self) -> bool {
		self.kind.starts_with("var_")
	}
}

/// Where a call in the unit sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
	Load,
	Setup,
	Step,
}

/// What one call did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallObservation {
	pub index: usize,
	/// The name asked for, and the one the lookup settled on.
	#[serde(default)]
	pub target: Option<Resolved>,
	pub outcome: Outcome,
	#[serde(default)]
	pub stdout: String,
	#[serde(default)]
	pub stdout_truncated: bool,
	#[serde(default)]
	pub files: BTreeMap<String, Option<String>>,
	#[serde(default)]
	pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
	pub requested: String,
	pub resolved: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
	Returned {
		value: Value,
		#[serde(rename = "type")]
		type_name: String,
	},
	Raised(ErrorInfo),
	Timeout {},
	Unserialisable(ErrorInfo),
	/// The target does not exist.
	Missing {
		message: String,
	},
	/// A `$ref` or `object` the call needs was never produced.
	Unresolved {
		name: String,
	},
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorInfo {
	#[serde(rename = "type")]
	pub type_name: String,
	/// The exception's MRO names, so a subclass satisfies `expect_error`.
	#[serde(default)]
	pub types: Vec<String>,
	#[serde(default)]
	pub message: String,
}

impl ErrorInfo {
	pub fn is_a(&self, name: &str) -> bool {
		self.type_name == name || self.types.iter().any(|t| t == name)
	}
}

/// What an in-process checker decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckObservation {
	Verdict {
		pass: bool,
		message: String,
	},
	/// The checker raised `AssertionError`: the answer is wrong.
	Rejected {
		message: String,
	},
	/// A name the checker takes was never produced — its producing call failed.
	Unresolved {
		name: String,
	},
	/// The checker could not decide.
	Error(ErrorInfo),
}

/// How the unit's process ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
	Code(i32),
	Signal(i32),
	/// Killed by the grader at the unit's deadline.
	Deadline,
	/// Never started.
	Spawn(String),
}

/// Everything a unit reported, in plan order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitObservation {
	/// Teacher modules imported and the harness running.
	pub ready: bool,
	pub load: Option<CallObservation>,
	pub setup: Vec<CallObservation>,
	pub steps: Vec<CallObservation>,
	pub checks: BTreeMap<usize, CheckObservation>,
	pub fatal: Option<Fatal>,
	pub done: bool,
	pub exit: Exit,
	/// The record stream broke: the unit's records cannot be trusted past this point.
	pub protocol_error: Option<ProtocolError>,
	/// The tail of stderr, for diagnosing a crash.
	pub stderr: String,
}

impl UnitObservation {
	/// An observation for a unit that never ran.
	pub fn not_started(exit: Exit) -> Self {
		Self {
			ready: false,
			load: None,
			setup: Vec::new(),
			steps: Vec::new(),
			checks: BTreeMap::new(),
			fatal: None,
			done: false,
			exit,
			protocol_error: None,
			stderr: String::new(),
		}
	}
}

/// How the record stream broke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolError {
	/// A framed record did not parse — the harness wrote something it should not have.
	Unreadable(String),
	/// A record repeated or came out of order — somebody else wrote it.
	Tampered(String),
	/// More reached the record channel than the harness ever writes: something other than
	/// the harness wrote to it.
	Flooded(String),
}

impl std::fmt::Display for ProtocolError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ProtocolError::Unreadable(m)
			| ProtocolError::Tampered(m)
			| ProtocolError::Flooded(m) => f.write_str(m),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fatal {
	pub stage: String,
	pub error: ErrorInfo,
}
