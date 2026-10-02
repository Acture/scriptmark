//! Frozen expectations and the exact source/configuration contract that produced them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::models::{TestCase, TestSpec};
use crate::runner::executor::Executor;

pub fn digest(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

/// A tagged return preserves `Some(null)` across JSON; an absent expectation is different.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedOutcome {
	Returned(Value),
	Raised(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
	pub outcome: Option<ExpectedOutcome>,
	pub stdout: Option<String>,
	pub files: BTreeMap<String, String>,
}

impl Answer {
	pub fn of(case: &TestCase) -> Self {
		Self {
			outcome: case
				.expect
				.clone()
				.map(ExpectedOutcome::Returned)
				.or_else(|| case.expect_error.clone().map(ExpectedOutcome::Raised)),
			stdout: case.expected_stdout.clone(),
			files: case.expect_files.clone(),
		}
	}

	pub fn apply(&self, case: &mut TestCase) {
		case.expect = None;
		case.expect_error = None;
		match &self.outcome {
			Some(ExpectedOutcome::Returned(value)) => case.expect = Some(value.clone()),
			Some(ExpectedOutcome::Raised(name)) => case.expect_error = Some(name.clone()),
			None => {}
		}
		case.expected_stdout = self.stdout.clone();
		case.expect_files = self.files.clone();
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
	pub configuration: Value,
	pub runtime: String,
	/// SHA-256 of each declared reference, helper and input file, keyed by absolute path.
	pub sources: BTreeMap<String, String>,
	pub protocol: String,
}

impl Contract {
	pub fn of<E: Executor>(spec: &TestSpec, executor: &E, timeout: u64) -> Result<Self, String> {
		let mut sources: BTreeMap<String, String> = BTreeMap::new();
		let mut paths: BTreeSet<String> = spec.meta.imports.iter().cloned().collect();
		paths.extend(
			spec.meta
				.data_files
				.iter()
				.map(|p| spec.dir.join(p).to_string_lossy().into_owned()),
		);
		for case in spec
			.cases
			.iter()
			.chain(spec.scenarios.iter().flat_map(|s| &s.steps))
		{
			if let Some(reference) = case.oracle.as_ref().and_then(|o| o.reference.as_ref()) {
				paths.insert(reference.clone());
			}
			if let Some(Ok(crate::models::Check::Python(script))) =
				case.check.as_ref().map(|c| c.resolve())
			{
				paths.insert(script);
			}
		}
		for path in paths {
			fingerprint(Path::new(&path), &mut sources, &mut BTreeSet::new())?;
		}
		Ok(Self {
			configuration: json!({"spec": spec, "timeout": timeout, "scriptmark": env!("CARGO_PKG_VERSION")}),
			runtime: executor.identity()?,
			sources,
			protocol: digest(
				concat!(
					include_str!("oracle.rs"),
					include_str!("harness.py"),
					include_str!("../checker/rhai_checker.rs")
				)
				.as_bytes(),
			),
		})
	}
}

pub(crate) fn fingerprint(
	path: &Path,
	files: &mut BTreeMap<String, String>,
	parents: &mut BTreeSet<std::path::PathBuf>,
) -> Result<(), String> {
	let failed = |e: std::io::Error| format!("cannot fingerprint '{}': {e}", path.display());
	let canonical = path.canonicalize().map_err(failed)?;
	if path.is_dir() {
		if !parents.insert(canonical.clone()) {
			return Err(format!(
				"source directory '{}' contains a symlink cycle",
				path.display()
			));
		}
		// Record even an empty directory so additions/removals invalidate replay.
		files.insert(path.to_string_lossy().into_owned(), "directory".into());
		for entry in std::fs::read_dir(path).map_err(failed)? {
			fingerprint(&entry.map_err(failed)?.path(), files, parents)?;
		}
		parents.remove(&canonical);
	} else {
		files.insert(
			path.to_string_lossy().into_owned(),
			digest(&std::fs::read(path).map_err(failed)?),
		);
	}
	Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenAnswers {
	pub contract: Contract,
	pub cases: BTreeMap<String, Answer>,
	/// Accidental edits or partial copies must not quietly produce different grades.
	pub checksum: String,
}

impl FrozenAnswers {
	pub fn new(contract: Contract, spec: &TestSpec) -> Self {
		let cases: BTreeMap<String, Answer> = spec
			.cases
			.iter()
			.filter(|c| c.oracle.as_ref().is_some_and(|o| o.computes()))
			.map(|c| (c.name.clone(), Answer::of(c)))
			.collect();
		let checksum: String = Self::checksum(&contract, &cases);
		Self {
			contract,
			cases,
			checksum,
		}
	}

	fn checksum(contract: &Contract, cases: &BTreeMap<String, Answer>) -> String {
		digest(&serde_json::to_vec(&(contract, cases)).expect("answer records are JSON values"))
	}

	pub fn restore(&self, contract: &Contract, spec: &mut TestSpec) -> Result<(), String> {
		const AGAIN: &str = "restore the original sources/configuration, or prepare a fresh bundle without --replay (use --fresh to replace an existing freeze)";
		if self.contract != *contract {
			return Err(format!(
				"frozen answers no longer match the inputs, sources, runtime or configuration: {AGAIN}"
			));
		}
		if self.checksum != Self::checksum(&self.contract, &self.cases) {
			return Err(format!("frozen answers failed their checksum: {AGAIN}"));
		}
		let expected: BTreeSet<&str> = spec
			.cases
			.iter()
			.filter(|c| c.oracle.as_ref().is_some_and(|o| o.computes()))
			.map(|c| c.name.as_str())
			.collect();
		if expected != self.cases.keys().map(String::as_str).collect() {
			return Err(format!(
				"frozen answers have missing or extra cases: {AGAIN}"
			));
		}
		for case in &mut spec.cases {
			if let Some(answer) = self.cases.get(&case.name) {
				answer.apply(case);
			}
		}
		Ok(())
	}
}
