//! The grading record: the evidence a batch was graded on, and every score revision reached
//! from it, in one versioned file.
//!
//! Evidence is what execution produced — each student's case results and matching
//! decisions, and the submissions and test bundle they came from. It is never rewritten. A
//! revision is that evidence scored under one policy. Rescoring appends a revision and runs
//! nothing, and it refuses evidence whose submissions, matching or tests have changed since:
//! that evidence no longer describes them.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::grading::{self, Policy};
use crate::matching::{self, ItemMatch};
use crate::models::{
	Assignment, AttemptPolicy, Check, FileOrigin, FileVersion, Grade, GradeOutcome, GradingConfig,
	GradingItem, Reason, StudentReport, StudentSubmission, SubmissionVersion, TestSpec,
};
use crate::runner::answers::{digest, fingerprint};
use crate::runner::frozen::{Frozen, scratch};

/// The layout of the file. A newer one is refused by number, never half-read.
pub const FORMAT: u32 = 1;

/// How many differences a refusal lists before it only counts the rest.
const SHOWN: usize = 20;

/// Evidence and the revisions scored from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
	pub format: u32,
	pub evidence: Evidence,
	/// SHA-256 of `evidence`. Revisions and database sessions name the evidence by it, and a
	/// record whose evidence no longer hashes to it is refused.
	pub digest: String,
	/// Oldest first, numbered from 1. Rescoring appends; nothing rewrites one.
	pub revisions: Vec<Revision>,
}

/// What a batch was graded on, and what running it found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
	/// The build that ran the students.
	pub scriptmark: String,
	pub assignment: AssignmentId,
	/// Where the batch's inputs were found; `rescore` looks there unless told otherwise.
	pub inputs: Inputs,
	pub attempt_policy: AttemptPolicy,
	pub matching: matching::Config,
	pub bundle: BundleVersion,
	/// One per student, sorted by `student_id`, ungraded.
	pub students: Vec<StudentReport>,
}

/// Which assignment this is, after the input settled it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentId {
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub canvas_course_id: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub canvas_assignment_id: Option<u64>,
}

impl From<&Assignment> for AssignmentId {
	fn from(assignment: &Assignment) -> Self {
		Self {
			name: assignment.name.clone(),
			canvas_course_id: assignment.canvas_course_id,
			canvas_assignment_id: assignment.canvas_assignment_id,
		}
	}
}

/// Where a batch's inputs are, as absolute paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
	pub tests: PathBuf,
	/// The `assignment.toml` read, when there was one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub assignment: Option<PathBuf>,
	pub source: Source,
}

/// Where the submissions came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
	Local {
		dirs: Vec<PathBuf>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		roster: Option<PathBuf>,
	},
	Canvas {
		bundle: PathBuf,
	},
}

/// The test bundle a batch ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleVersion {
	/// SHA-256 over the timeout and every spec's digest and sources, in order: the bundle's
	/// version.
	pub digest: String,
	/// The `--timeout` every call ran under, where a spec did not set its own.
	pub timeout: u64,
	/// The interpreter that ran the students. Provenance only: rescoring never runs it.
	pub python: String,
	/// In load order, which is also the order lint looks for a `[lint]` spec in.
	pub specs: Vec<SpecVersion>,
	/// The frozen inputs and answers the batch used, when it had any.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub frozen: Option<FrozenLink>,
}

/// One test spec as it ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecVersion {
	pub name: String,
	/// SHA-256 of the spec as loaded: every case, check, timeout and path it declares.
	pub digest: String,
	/// SHA-256 of every teacher file the spec reads, by absolute path.
	pub sources: BTreeMap<String, String>,
	/// Each template's seed, by template name.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub seeds: BTreeMap<String, u64>,
	/// The checksum of the spec's frozen reference answers, when it computes any.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub answers: Option<String>,
}

/// Frozen inputs and answers, linked rather than copied: their file is where `--replay`
/// reads them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenLink {
	/// The file holding them, when there is one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<PathBuf>,
	/// SHA-256 of their JSON, as `Frozen::to_json` writes it.
	pub digest: String,
}

/// The evidence scored under one policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
	pub revision: u32,
	/// The build that scored it: a grading fix can change grades under an identical policy.
	pub scriptmark: String,
	/// The evidence it was scored from: the record's `digest`.
	pub evidence: String,
	pub policy: ScoringPolicy,
	/// One per student, by `student_id`.
	pub grades: BTreeMap<String, Grade>,
	/// Accidental edits must not quietly publish other grades.
	pub checksum: String,
}

/// Everything a score depends on besides the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoringPolicy {
	pub items: Vec<GradingItem>,
	pub grading: GradingConfig,
	pub derived_items: bool,
}

/// The reports as one revision scored them.
#[derive(Debug, Clone)]
pub struct View {
	/// `None` for evidence nothing has scored yet.
	pub revision: Option<u32>,
	/// How many revisions the record holds.
	pub of: usize,
	pub reports: Vec<StudentReport>,
	/// The items the revision scored, in declaration order; empty when unscored.
	pub items: Vec<GradingItem>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
	#[error("cannot read grading record {path}: {error}", path = .0.display(), error = .1)]
	Io(PathBuf, std::io::Error),
	#[error("{path} is not a grading record this build can read: {why}", path = .0.display(), why = .1)]
	Invalid(PathBuf, String),
}

impl Record {
	/// Unscored evidence. Students are sorted by id; two with one id, or one already graded,
	/// are refused: a revision could not tell them apart.
	pub fn new(mut evidence: Evidence) -> Result<Record> {
		evidence
			.students
			.sort_by(|a, b| a.student_id.cmp(&b.student_id));
		if let Some(pair) = evidence
			.students
			.windows(2)
			.find(|pair| pair[0].student_id == pair[1].student_id)
		{
			bail!(
				"two reports share student id '{}'; a grading record cannot tell them apart",
				pair[0].student_id
			);
		}
		if let Some(report) = evidence.students.iter().find(|r| r.grade.is_some()) {
			bail!(
				"the evidence for '{}' already carries a grade",
				report.student_id
			);
		}
		let digest = evidence_digest(&evidence).context("the evidence cannot be recorded")?;
		Ok(Record {
			format: FORMAT,
			evidence,
			digest,
			revisions: Vec::new(),
		})
	}

	/// The format first, then the strict body, so a newer record is refused by its number
	/// rather than by a field this build has never heard of — and a results list from before
	/// grading records by what it is.
	pub fn from_json(text: &str) -> Result<Record, String> {
		let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
		if value.is_array() {
			return Err(
				"it is a list of reports from before grading records; grade the submissions again"
					.into(),
			);
		}
		match value.get("format").and_then(Value::as_u64) {
			None => Err("it has no format".into()),
			Some(n) if n == u64::from(FORMAT) => {
				let record: Record = serde_json::from_value(value).map_err(|e| e.to_string())?;
				record.verify()?;
				Ok(record)
			}
			Some(other) => Err(format!(
				"it is format {other}, and this build reads format {FORMAT}"
			)),
		}
	}

	pub fn load(path: &Path) -> Result<Record, RecordError> {
		let text =
			std::fs::read_to_string(path).map_err(|e| RecordError::Io(path.to_path_buf(), e))?;
		Record::from_json(&text).map_err(|e| RecordError::Invalid(path.to_path_buf(), e))
	}

	pub fn to_json(&self) -> String {
		// `new` already serialised the evidence, and a revision is numbers and words.
		serde_json::to_string_pretty(self).expect("a grading record is plain JSON") + "\n"
	}

	/// Write through a temporary file and a rename: the record is the only copy of its
	/// revisions, and a half-written one would lose them all.
	pub fn write(&self, path: &Path) -> std::io::Result<()> {
		let mut file = scratch(path)?;
		file.write_all(self.to_json().as_bytes())?;
		file.persist(path).map_err(|e| e.error)?;
		Ok(())
	}

	/// What the stored digests and numbering claim must hold.
	fn verify(&self) -> Result<(), String> {
		if evidence_digest(&self.evidence).map_err(|e| e.to_string())? != self.digest {
			return Err(
				"its evidence does not match its digest: it was edited or is incomplete".into(),
			);
		}
		let students = &self.evidence.students;
		if students
			.windows(2)
			.any(|pair| pair[0].student_id >= pair[1].student_id)
		{
			return Err("its students are not one per id, in order".into());
		}
		if let Some(report) = students.iter().find(|r| r.grade.is_some()) {
			return Err(format!(
				"the evidence for '{}' carries a grade",
				report.student_id
			));
		}
		let ids: BTreeSet<&str> = students.iter().map(|r| r.student_id.as_str()).collect();
		for (i, revision) in self.revisions.iter().enumerate() {
			let n = revision.revision;
			if usize::try_from(n).ok() != Some(i + 1) {
				return Err("its revisions are not numbered from 1 in order".into());
			}
			if revision.evidence != self.digest {
				return Err(format!("revision {n} was scored from other evidence"));
			}
			if revision.checksum != revision_checksum(revision) {
				return Err(format!("revision {n} failed its checksum"));
			}
			if revision
				.grades
				.keys()
				.map(String::as_str)
				.collect::<BTreeSet<_>>()
				!= ids
			{
				return Err(format!(
					"revision {n} does not grade exactly the record's students"
				));
			}
		}
		Ok(())
	}

	pub fn latest(&self) -> Option<&Revision> {
		self.revisions.last()
	}

	pub fn revision(&self, n: u32) -> Result<&Revision> {
		usize::try_from(n)
			.ok()
			.and_then(|n| n.checked_sub(1))
			.and_then(|i| self.revisions.get(i))
			.with_context(|| match self.revisions.len() {
				0 => format!("there is no revision {n}: nothing has scored this evidence yet"),
				len => format!("there is no revision {n}: the record holds revisions 1 to {len}"),
			})
	}

	/// The reports as revision `n` scored them: the latest when `n` is `None`, and unscored
	/// evidence when nothing has scored it yet.
	pub fn view(&self, n: Option<u32>) -> Result<View> {
		let revision = match n {
			Some(n) => Some(self.revision(n)?),
			None => self.latest(),
		};
		let mut reports = self.evidence.students.clone();
		if let Some(revision) = revision {
			for report in &mut reports {
				report.grade = revision.grades.get(&report.student_id).cloned();
			}
		}
		Ok(View {
			revision: revision.map(|r| r.revision),
			of: self.revisions.len(),
			reports,
			items: revision.map(|r| r.policy.items.clone()).unwrap_or_default(),
		})
	}

	/// Score the evidence under `policy` as a new revision, and return its number. Runs
	/// nothing. Refused when the latest revision already used this policy on this build: it
	/// would only repeat it.
	pub fn score(&mut self, items: &[GradingItem], policy: &Policy) -> Result<u32> {
		let scoring = ScoringPolicy {
			items: items.to_vec(),
			grading: policy.config().clone(),
			derived_items: policy.derived_items(),
		};
		let build = env!("CARGO_PKG_VERSION");
		if let Some(latest) = self.latest()
			&& latest.policy == scoring
			&& latest.scriptmark == build
		{
			bail!(
				"nothing to rescore: revision {} already scored this evidence under the same policy",
				latest.revision
			);
		}
		let mut reports = self.evidence.students.clone();
		grading::grade_all(&mut reports, items, policy)?;
		let grades = reports
			.into_iter()
			.map(|r| {
				let grade = r.grade.expect("grade_all grades every report");
				(r.student_id, grade)
			})
			.collect();
		let n = u32::try_from(self.revisions.len() + 1).context("too many revisions")?;
		let mut revision = Revision {
			revision: n,
			scriptmark: build.to_string(),
			evidence: self.digest.clone(),
			policy: scoring,
			grades,
			checksum: String::new(),
		};
		revision.checksum = revision_checksum(&revision);
		self.revisions.push(revision);
		Ok(n)
	}

	/// Whether the evidence still describes the batch as it is now: the same assignment,
	/// submissions, matching and tests. Only then may a new policy score it. Refuses with
	/// every difference found. Reads files; runs nothing.
	pub fn check(&self, current: &Current) -> Result<()> {
		let evidence = &self.evidence;
		let mut changed = Vec::new();
		// The name is a label — the record keeps the one it was graded under — but the
		// Canvas ids say whose grades these are.
		let (was, now) = (&evidence.assignment, AssignmentId::from(current.assignment));
		if (now.canvas_course_id, now.canvas_assignment_id)
			!= (was.canvas_course_id, was.canvas_assignment_id)
		{
			changed.push(format!(
				"the assignment's Canvas ids are {}, not {}",
				canvas_ids(&now),
				canvas_ids(was)
			));
		}
		if current.attempt_policy != evidence.attempt_policy {
			changed.push(format!(
				"the attempt policy is {}, not {}",
				word(&current.attempt_policy),
				word(&evidence.attempt_policy)
			));
		}
		if *current.matching != evidence.matching {
			changed.push("the [matching] rules changed".into());
		}
		let specs = spec_versions(current.specs)?;
		let same_specs = spec_changes(&evidence.bundle.specs, &specs, &mut changed);
		student_changes(evidence, current, same_specs, &mut changed)?;
		if changed.is_empty() {
			return Ok(());
		}
		let more = changed.len().saturating_sub(SHOWN);
		changed.truncate(SHOWN);
		if more > 0 {
			changed.push(format!("… and {more} more"));
		}
		bail!(
			"refusing to rescore: the evidence no longer describes this batch, so only grading it \
			 again can:\n  {}",
			changed.join("\n  ")
		)
	}
}

/// A batch as it stands now, found without running anything.
pub struct Current<'a> {
	pub assignment: &'a Assignment,
	pub attempt_policy: AttemptPolicy,
	pub matching: &'a matching::Config,
	pub specs: &'a [TestSpec],
	pub students: &'a [StudentSubmission],
}

/// Records the spec differences; whether the specs are the same ones, in the same order.
fn spec_changes(recorded: &[SpecVersion], now: &[SpecVersion], changed: &mut Vec<String>) -> bool {
	let names = |specs: &[SpecVersion]| specs.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
	if names(recorded) != names(now) {
		changed.push(format!(
			"the test specs are [{}], not [{}]",
			names(now).join(", "),
			names(recorded).join(", ")
		));
		return false;
	}
	for (was, now) in recorded.iter().zip(now) {
		if was.digest != now.digest {
			changed.push(format!("test spec '{}' changed", now.name));
		}
		let paths: BTreeSet<&String> = was.sources.keys().chain(now.sources.keys()).collect();
		for path in paths {
			if was.sources.get(path) != now.sources.get(path) {
				changed.push(format!(
					"{path}, which test spec '{}' reads, changed",
					now.name
				));
			}
		}
	}
	true
}

fn student_changes(
	evidence: &Evidence,
	current: &Current,
	same_specs: bool,
	changed: &mut Vec<String>,
) -> Result<()> {
	let recorded: BTreeMap<&str, &StudentReport> = evidence
		.students
		.iter()
		.map(|r| (r.student_id.as_str(), r))
		.collect();
	let now: BTreeMap<String, &StudentSubmission> = current
		.students
		.iter()
		.map(|s| (s.key().to_string(), s))
		.collect();
	for id in recorded.keys().filter(|id| !now.contains_key(**id)) {
		changed.push(format!("student {id} is no longer in the input"));
	}
	for (id, student) in &now {
		let Some(report) = recorded.get(id.as_str()) else {
			changed.push(format!("student {id} was not graded"));
			continue;
		};
		if report.canvas_user_id != student.identity.canvas_user_id {
			changed.push(format!("{id}'s Canvas user id changed"));
		}
		if report.submission_state != student.outcome() {
			changed.push(format!(
				"{id}'s submission is now {}, not {}",
				word(&student.outcome()),
				word(&report.submission_state)
			));
		}
		if report.excused != student.is_excused() {
			changed.push(format!(
				"{id} is {}excused now",
				if student.is_excused() {
					""
				} else {
					"no longer "
				}
			));
		}
		if report.submission.as_ref() != Some(&submission_version(student)?) {
			changed.push(format!("{id}'s submitted files changed"));
		}
		if same_specs {
			for spec in current.specs {
				let now = matching::item_match(current.matching, student, spec);
				let was = report.matches.iter().find(|m| m.item == now.item);
				if !was.is_some_and(|was| same_static_match(was, &now)) {
					changed.push(format!(
						"{id}'s file for '{}' is matched differently",
						now.item
					));
				}
			}
		}
	}
	Ok(())
}

/// The decisions made before anything ran. Function decisions are made while running and
/// are evidence, not something to compare against.
fn same_static_match(a: &ItemMatch, b: &ItemMatch) -> bool {
	a.student == b.student
		&& a.item == b.item
		&& a.file == b.file
		&& a.origin == b.origin
		&& a.owner == b.owner
}

/// Each spec's version as loaded, in load order. Reads files; runs nothing.
pub fn spec_versions(specs: &[TestSpec]) -> Result<Vec<SpecVersion>> {
	specs
		.iter()
		.map(|spec| {
			Ok(SpecVersion {
				name: spec.meta.name.clone(),
				digest: digest(&serde_json::to_vec(spec)?),
				sources: sources(spec).map_err(anyhow::Error::msg)?,
				seeds: BTreeMap::new(),
				answers: None,
			})
		})
		.collect()
}

/// Every teacher file a spec reads, hashed: imports, data files, Python checkers and
/// reference implementations, and every other `.py` file beside a module, checker or
/// reference — the harness puts their directory on `sys.path`, so a helper there is
/// imported without being declared. A Finder `.DS_Store` in a data directory is not.
fn sources(spec: &TestSpec) -> Result<BTreeMap<String, String>, String> {
	let mut files: BTreeSet<PathBuf> = spec
		.meta
		.data_files
		.iter()
		.map(|p| spec.dir.join(p))
		.collect();
	let mut python: BTreeSet<PathBuf> = spec.meta.imports.iter().map(PathBuf::from).collect();
	for case in spec
		.cases
		.iter()
		.chain(spec.scenarios.iter().flat_map(|s| &s.steps))
	{
		if let Some(Ok(Check::Python(script))) = case.check.as_ref().map(|c| c.resolve()) {
			python.insert(script.into());
		}
		for oracle in case
			.oracle
			.iter()
			.chain(case.parametrize.iter().map(|p| &p.oracle))
		{
			if let Some(reference) = &oracle.reference {
				python.insert(reference.into());
			}
		}
	}
	let dirs: BTreeSet<&Path> = python
		.iter()
		.map(|module| {
			module
				.parent()
				.filter(|p| !p.as_os_str().is_empty())
				.unwrap_or(Path::new("."))
		})
		.collect();
	for dir in dirs {
		let failed = |e: std::io::Error| format!("cannot list '{}': {e}", dir.display());
		for entry in std::fs::read_dir(dir).map_err(failed)? {
			let path = entry.map_err(failed)?.path();
			if path.extension().is_some_and(|e| e == "py") && path.is_file() {
				files.insert(path);
			}
		}
	}
	files.extend(python);
	let mut hashes = BTreeMap::new();
	for path in files {
		fingerprint(&path, &mut hashes, &mut BTreeSet::new())?;
	}
	// Finder leaves these in any folder it opens; no test reads one.
	hashes.retain(|path, _| !path.ends_with("/.DS_Store"));
	Ok(hashes)
}

/// The bundle a batch ran: each spec's version with the seeds and answers it was prepared
/// with, and a link to the frozen file that holds them.
pub fn bundle_version(
	mut specs: Vec<SpecVersion>,
	timeout: u64,
	python: String,
	frozen: &Frozen,
	frozen_path: Option<PathBuf>,
) -> BundleVersion {
	for spec in &mut specs {
		if let Some(templates) = frozen.specs.get(&spec.name) {
			spec.seeds = templates
				.iter()
				.filter_map(|(name, made)| made.seed.map(|seed| (name.clone(), seed)))
				.collect();
		}
		spec.answers = frozen.answers.get(&spec.name).map(|a| a.checksum.clone());
	}
	let semantics: Vec<(&String, &String, &BTreeMap<String, String>)> = specs
		.iter()
		.map(|s| (&s.name, &s.digest, &s.sources))
		.collect();
	BundleVersion {
		digest: digest(&serde_json::to_vec(&(timeout, semantics)).expect("plain JSON")),
		timeout,
		python,
		frozen: (!frozen.is_empty()).then(|| FrozenLink {
			path: frozen_path,
			digest: digest(frozen.to_json().as_bytes()),
		}),
		specs,
	}
}

/// What `student` hands in now: the selected attempt, the SHA-256 of each of its files, and
/// of the archives they came out of.
pub fn submission_version(student: &StudentSubmission) -> Result<SubmissionVersion> {
	let hash = |path: &Path| -> Result<FileVersion> {
		// Taken before anything runs: a path the record cannot hold must not cost a run.
		if path.to_str().is_none() {
			bail!("cannot record {}: its path is not UTF-8", path.display());
		}
		let bytes =
			std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
		Ok(FileVersion {
			path: path.to_path_buf(),
			sha256: digest(&bytes),
		})
	};
	let mut files = student
		.files()
		.iter()
		.map(|f| hash(&f.path))
		.collect::<Result<Vec<_>>>()?;
	files.sort();
	let archives: BTreeSet<&PathBuf> = student
		.files()
		.iter()
		.filter_map(|f| match &f.origin {
			FileOrigin::Archive { archive, .. } => Some(archive),
			_ => None,
		})
		.collect();
	let attempt = student.selected_attempt();
	Ok(SubmissionVersion {
		attempt: attempt.map(|a| a.attempt),
		submitted_at: attempt.and_then(|a| a.submitted_at.clone()),
		files,
		archives: archives
			.into_iter()
			.map(|p| hash(p))
			.collect::<Result<_>>()?,
	})
}

/// Each student's submission version, in input order.
pub fn submission_versions(students: &[StudentSubmission]) -> Result<Vec<SubmissionVersion>> {
	students.iter().map(submission_version).collect()
}

/// Put on each report — `run_all` returns them in input order — the version its student's
/// submission had before the run, refusing when one changed meanwhile: its cases may have
/// judged two versions.
pub fn seal(
	reports: &mut [StudentReport],
	students: &[StudentSubmission],
	before: Vec<SubmissionVersion>,
) -> Result<()> {
	let after = submission_versions(students)?;
	if reports.len() != students.len() || before.len() != students.len() {
		bail!("the reports do not match the students one to one");
	}
	for ((report, student), (was, now)) in reports
		.iter_mut()
		.zip(students)
		.zip(before.into_iter().zip(after))
	{
		if report.student_id != student.key().to_string() {
			bail!(
				"the report for '{}' is not in its student's place",
				report.student_id
			);
		}
		if was != now {
			bail!(
				"{}'s submission changed while it was being graded; grade again",
				report.student_id
			);
		}
		report.submission = Some(was);
	}
	Ok(())
}

/// Whether a run may write its record over `path` without losing anything: not when it
/// holds rescored revisions, grades kept nowhere else. Anything else is replaced, as before.
/// The caller says what to do instead, and may be told to replace it anyway.
pub fn check_replaceable(path: &Path) -> Result<(), String> {
	let Ok(text) = std::fs::read_to_string(path) else {
		return Ok(());
	};
	match Record::from_json(&text) {
		Ok(record) if record.revisions.len() > 1 => Err(format!(
			"{} holds {} score revisions, which it alone records",
			path.display(),
			record.revisions.len()
		)),
		_ => Ok(()),
	}
}

/// How a grade stands, in a few words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeState {
	Graded,
	Withheld,
}

/// A grade and the numbers behind it, for comparing two revisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Standing {
	pub state: GradeState,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reason: Option<Reason>,
	pub max: f64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub score: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub raw_grade: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub final_grade: Option<f64>,
}

impl Standing {
	pub fn of(grade: &Grade) -> Self {
		let (state, score, raw_grade) = match grade.outcome {
			GradeOutcome::Graded {
				score, raw_grade, ..
			} => (GradeState::Graded, Some(score), Some(raw_grade)),
			GradeOutcome::Withheld { .. } => (GradeState::Withheld, None, None),
		};
		Self {
			state,
			reason: grade.reason(),
			max: grade.max,
			score,
			raw_grade,
			final_grade: grade.final_grade(),
		}
	}
}

/// One student whose grade differs between two revisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
	pub student_id: String,
	/// `None` when there was no earlier revision.
	pub before: Option<Standing>,
	pub after: Standing,
}

/// The students whose grade `after` changed from `before`, by id.
pub fn diff(before: Option<&Revision>, after: &Revision) -> Vec<Change> {
	after
		.grades
		.iter()
		.filter_map(|(id, grade)| {
			let now = Standing::of(grade);
			let was = before.and_then(|b| b.grades.get(id)).map(Standing::of);
			(was.as_ref() != Some(&now)).then(|| Change {
				student_id: id.clone(),
				before: was,
				after: now,
			})
		})
		.collect()
}

fn evidence_digest(evidence: &Evidence) -> serde_json::Result<String> {
	Ok(digest(&serde_json::to_vec(evidence)?))
}

fn revision_checksum(revision: &Revision) -> String {
	digest(
		&serde_json::to_vec(&(
			revision.revision,
			&revision.scriptmark,
			&revision.evidence,
			&revision.policy,
			&revision.grades,
		))
		.expect("a revision is plain JSON"),
	)
}

fn canvas_ids(assignment: &AssignmentId) -> String {
	let id = |id: Option<u64>| id.map_or_else(|| "none".to_string(), |id| id.to_string());
	format!(
		"course {} and assignment {}",
		id(assignment.canvas_course_id),
		id(assignment.canvas_assignment_id)
	)
}

fn word<T: Serialize>(value: &T) -> String {
	crate::export::word(value)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{CaseResult, Cause, Fault, SubmissionOutcome, TestResult, TestStatus};

	/// A student with `passed` and `failed` cases on the one item, `sum`.
	fn report(id: &str, passed: usize, failed: usize) -> StudentReport {
		let case = |name: String, status: TestStatus| CaseResult {
			case_name: name,
			status,
			fault: (status != TestStatus::Passed).then_some(Fault::Student),
			cause: (status != TestStatus::Passed).then_some(Cause::Wrong),
			..Default::default()
		};
		let cases = (0..passed)
			.map(|i| case(format!("pass {i}"), TestStatus::Passed))
			.chain((0..failed).map(|i| case(format!("fail {i}"), TestStatus::Failed)))
			.collect();
		StudentReport {
			test_results: vec![TestResult {
				item_id: "sum".into(),
				file: None,
				cases,
			}],
			..StudentReport::new(id, SubmissionOutcome::Executable)
		}
	}

	fn evidence(students: Vec<StudentReport>) -> Evidence {
		let frozen = Frozen {
			format: crate::runner::frozen::FORMAT,
			scriptmark: "test".into(),
			specs: BTreeMap::new(),
			answers: BTreeMap::new(),
		};
		Evidence {
			scriptmark: "test".into(),
			assignment: AssignmentId {
				name: "hw".into(),
				canvas_course_id: None,
				canvas_assignment_id: None,
			},
			inputs: Inputs {
				tests: "/hw/tests".into(),
				assignment: None,
				source: Source::Local {
					dirs: vec!["/hw/submissions".into()],
					roster: None,
				},
			},
			attempt_policy: AttemptPolicy::default(),
			matching: matching::Config::default(),
			bundle: bundle_version(Vec::new(), 10, "python3".into(), &frozen, None),
			students,
		}
	}

	fn record() -> Record {
		Record::new(evidence(vec![report("bob", 1, 1), report("alice", 2, 0)])).unwrap()
	}

	/// One item, `sum`, worth `points`.
	fn policy(points: u32) -> (Vec<GradingItem>, Policy) {
		(
			vec![GradingItem {
				points,
				..GradingItem::new("sum")
			}],
			Policy::compile(GradingConfig::default(), false).unwrap(),
		)
	}

	fn rescored(record: &mut Record, points: u32) -> u32 {
		let (items, policy) = policy(points);
		record.score(&items, &policy).unwrap()
	}

	#[test]
	fn a_record_round_trips_with_every_revision() {
		let mut record = record();
		assert_eq!(rescored(&mut record, 10), 1);
		assert_eq!(rescored(&mut record, 20), 2);
		let read = Record::from_json(&record.to_json()).unwrap();
		assert_eq!(read.digest, record.digest);
		assert_eq!(read.revisions, record.revisions);
		assert_eq!(read.to_json(), record.to_json());
	}

	#[test]
	fn students_are_kept_in_id_order_and_never_twice() {
		assert_eq!(
			record()
				.evidence
				.students
				.iter()
				.map(|r| r.student_id.as_str())
				.collect::<Vec<_>>(),
			["alice", "bob"]
		);
		let err =
			Record::new(evidence(vec![report("bob", 1, 0), report("bob", 0, 1)])).unwrap_err();
		assert!(err.to_string().contains("share student id 'bob'"), "{err}");
	}

	#[test]
	fn evidence_carrying_a_grade_is_not_evidence() {
		let mut record = record();
		rescored(&mut record, 10);
		let graded = record.view(None).unwrap().reports;
		let err = Record::new(evidence(graded)).unwrap_err();
		assert!(err.to_string().contains("already carries a grade"), "{err}");
	}

	#[test]
	fn an_edited_record_is_refused() {
		let mut record = record();
		rescored(&mut record, 10);
		let edit = |change: &dyn Fn(&mut Value)| {
			let mut value: Value = serde_json::from_str(&record.to_json()).unwrap();
			change(&mut value);
			Record::from_json(&value.to_string()).unwrap_err()
		};

		let err = edit(&|v| {
			v["evidence"]["students"][1]["test_results"][0]["cases"][1]["status"] = "passed".into()
		});
		assert!(err.contains("does not match its digest"), "{err}");

		let err = edit(&|v| v["revisions"][0]["grades"]["bob"]["final_grade"] = 100.0.into());
		assert!(err.contains("revision 1 failed its checksum"), "{err}");

		let err = edit(&|v| v["revisions"][0]["revision"] = 2.into());
		assert!(err.contains("not numbered from 1"), "{err}");

		let err = edit(&|v| {
			v["evidence"]["students"][0]["grade"] = v["revisions"][0]["grades"]["alice"].clone()
		});
		assert!(err.contains("does not match its digest"), "{err}");
	}

	#[test]
	fn results_from_before_grading_records_and_newer_records_are_refused_by_what_they_are() {
		let err = Record::from_json("[]").unwrap_err();
		assert!(err.contains("before grading records"), "{err}");

		let mut value: Value = serde_json::from_str(&record().to_json()).unwrap();
		value["format"] = 2.into();
		value["something_new"] = true.into();
		let err = Record::from_json(&value.to_string()).unwrap_err();
		assert!(err.contains("it is format 2"), "{err}");

		let err = Record::from_json("{}").unwrap_err();
		assert!(err.contains("no format"), "{err}");
	}

	#[test]
	fn rescoring_appends_and_refuses_to_repeat_the_latest_revision() {
		let mut record = record();
		rescored(&mut record, 10);
		let (items, policy) = policy(10);
		let err = record.score(&items, &policy).unwrap_err();
		assert!(
			err.to_string()
				.contains("revision 1 already scored this evidence"),
			"{err}"
		);
		rescored(&mut record, 20);
		// Going back to an earlier policy is a new decision, and a new revision.
		assert_eq!(rescored(&mut record, 10), 3);
		assert_eq!(record.revisions.len(), 3);
		assert!(record.revisions.iter().all(|r| r.evidence == record.digest));
	}

	#[test]
	fn a_view_shows_one_revision_and_unscored_evidence_before_any() {
		let mut record = record();
		let unscored = record.view(None).unwrap();
		assert_eq!((unscored.revision, unscored.of), (None, 0));
		assert!(unscored.reports.iter().all(|r| r.grade.is_none()));
		assert!(unscored.items.is_empty());
		let err = record.view(Some(1)).unwrap_err();
		assert!(err.to_string().contains("nothing has scored"), "{err}");

		rescored(&mut record, 10);
		rescored(&mut record, 20);
		let grade = |view: &View, id: &str| {
			view.reports
				.iter()
				.find(|r| r.student_id == id)
				.unwrap()
				.final_grade()
		};
		let first = record.view(Some(1)).unwrap();
		let latest = record.view(None).unwrap();
		assert_eq!(
			(first.revision, latest.revision, latest.of),
			(Some(1), Some(2), 2)
		);
		assert_eq!(first.items[0].points, 10);
		assert_eq!(latest.items[0].points, 20);
		assert_eq!(grade(&first, "bob"), Some(50.0));
		assert_eq!(grade(&latest, "bob"), Some(50.0));
		let err = record.view(Some(3)).unwrap_err();
		assert!(err.to_string().contains("revisions 1 to 2"), "{err}");
	}

	#[test]
	fn a_diff_names_every_student_whose_grade_moved() {
		let mut record = record();
		rescored(&mut record, 10);
		let first = diff(None, &record.revisions[0]);
		assert_eq!(first.len(), 2, "everyone is new in the first revision");
		assert!(first.iter().all(|c| c.before.is_none()));

		// Points alone do not move a proportional grade, but they move the score.
		rescored(&mut record, 20);
		let changes = diff(Some(&record.revisions[0]), &record.revisions[1]);
		assert_eq!(changes.len(), 2);
		let bob = changes.iter().find(|c| c.student_id == "bob").unwrap();
		assert_eq!(bob.before.as_ref().unwrap().score, Some(5.0));
		assert_eq!(bob.after.score, Some(10.0));
		assert_eq!(bob.after.final_grade, Some(50.0));

		let (items, _) = policy(20);
		let all_or_nothing = Policy::compile(GradingConfig::default(), false).unwrap();
		let items: Vec<GradingItem> = items
			.into_iter()
			.map(|i| GradingItem {
				aggregation: crate::models::Aggregation::AllOrNothing,
				..i
			})
			.collect();
		record.score(&items, &all_or_nothing).unwrap();
		let changes = diff(Some(&record.revisions[1]), &record.revisions[2]);
		assert_eq!(
			changes
				.iter()
				.map(|c| (c.student_id.as_str(), c.after.final_grade))
				.collect::<Vec<_>>(),
			[("bob", Some(0.0))],
			"alice passed everything either way"
		);
	}

	#[test]
	fn a_record_with_rescored_revisions_is_never_replaced() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("results.json");
		assert!(check_replaceable(&path).is_ok(), "nothing there");
		std::fs::write(&path, "not a record").unwrap();
		assert!(check_replaceable(&path).is_ok());

		let mut record = record();
		record.write(&path).unwrap();
		assert!(check_replaceable(&path).is_ok(), "unscored");
		rescored(&mut record, 10);
		record.write(&path).unwrap();
		assert!(check_replaceable(&path).is_ok(), "only its own first grade");
		rescored(&mut record, 20);
		record.write(&path).unwrap();
		let err = check_replaceable(&path).unwrap_err();
		assert!(err.contains("holds 2 score revisions"), "{err}");
	}

	#[test]
	fn sources_cover_references_checkers_data_and_undeclared_helpers() {
		let dir = tempfile::tempdir().unwrap();
		let at = |name: &str, content: &str| {
			let path = dir.path().join(name);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, content).unwrap();
		};
		at("teacher/support.py", "from helper import X\n");
		at("teacher/helper.py", "X = 1\n");
		at("teacher/notes.txt", "not python");
		at("solutions/ref.py", "def answer(x):\n    return x\n");
		at("check.py", "print('{}')\n");
		at("data/poem.txt", "words\n");
		at("data/.DS_Store", "finder");
		let spec = crate::spec_loader::load_spec_str(
			"[meta]\nname = 'q'\nfile = 'q.py'\nfunction = 'f'\nlanguage = 'python'\n\
			 imports = ['teacher/support.py']\ndata_files = ['data/']\n\
			 [[cases]]\nname = 'one'\nargs = [1]\nexpect = 1\ncheck = { python = 'check.py' }\n\
			 [[cases]]\nname = 'many'\n[cases.parametrize]\nargs = { x = 'int(0, 3)' }\n\
			 samples = [[1]]\n[cases.parametrize.oracle]\nreference = 'solutions/ref.py'\n\
			 function = 'answer'\n",
			dir.path(),
		)
		.unwrap();
		let hashed: Vec<String> = sources(&spec)
			.unwrap()
			.into_keys()
			.map(|p| {
				p.strip_prefix(&*dir.path().to_string_lossy())
					.unwrap()
					.to_string()
			})
			.collect();
		for expected in [
			"/teacher/support.py",
			"/teacher/helper.py",
			"/solutions/ref.py",
			"/check.py",
			"/data/poem.txt",
		] {
			assert!(
				hashed.iter().any(|p| p == expected),
				"{expected} not in {hashed:?}"
			);
		}
		assert!(
			!hashed.iter().any(|p| p.ends_with("notes.txt")),
			"{hashed:?}"
		);
	}
}
