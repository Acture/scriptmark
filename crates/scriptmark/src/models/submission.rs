//! The unified input model.
//!
//! Both entry points — Canvas ([`crate::input::canvas`]) and local
//! ([`crate::discovery`]) — produce an [`AssignmentInput`], and everything downstream
//! reads only from it. Entry-specific material (Canvas workflow states, attachment URLs,
//! spreadsheet coordinates) is kept in clearly marked optional side fields so that it
//! never reaches the scoring path.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::config::AttemptPolicy;
use crate::roster::Roster;

/// The only normalisation applied to an identity key.
///
/// Strips a UTF-8 BOM and surrounding whitespace. Deliberately does *not* case-fold,
/// parse as a number, or touch zero padding — `"012345"` and `"12345"` are different
/// students and must stay that way.
pub fn normalize_key(raw: &str) -> String {
	raw.strip_prefix('\u{feff}')
		.unwrap_or(raw)
		.trim()
		.to_string()
}

/// Strip leading zeros, for *comparison only*. Never used to build a key.
pub fn zero_stripped(key: &str) -> &str {
	let trimmed = key.trim_start_matches('0');
	if trimmed.is_empty() { key } else { trimmed }
}

/// The total identity key for a student.
///
/// Every student in an [`AssignmentInput`] has exactly one, so a student can never be
/// keyless. The `Display` form is what lands in `StudentReport.student_id`, the database
/// and the CSV export — the prefixes make a Canvas id or an unconfirmed local token
/// impossible to mistake for a 学号.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudentKey {
	/// A confirmed 学号 — matched in the roster, or taken from a Canvas `sis_user_id`.
	Number(String),
	/// A Canvas user carrying no SIS id.
	CanvasUser(u64),
	/// A token pulled off a local filename that no roster confirms.
	Extracted(String),
}

impl fmt::Display for StudentKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Number(number) => write!(f, "{number}"),
			Self::CanvasUser(id) => write!(f, "canvas:{id}"),
			Self::Extracted(token) => write!(f, "local:{token}"),
		}
	}
}

/// Prefixes [`StudentKey`]'s `Display` uses to mark a Canvas-native or unconfirmed key.
/// A 学号 may not begin with one, or the rendering would stop being reversible.
pub const RESERVED_KEY_PREFIXES: [&str; 2] = ["local:", "canvas:"];

/// Whether this text could be mistaken for a rendered key of another kind.
pub fn is_reserved_key(text: &str) -> bool {
	RESERVED_KEY_PREFIXES
		.iter()
		.any(|prefix| text.starts_with(prefix))
}

impl StudentKey {
	/// The inverse of [`fmt::Display`], for reading a key back out of a results file, a
	/// database row or a CSV column.
	pub fn parse(rendered: &str) -> Self {
		if let Some(id) = rendered.strip_prefix("canvas:")
			&& let Ok(id) = id.parse::<u64>()
		{
			return Self::CanvasUser(id);
		}
		match rendered.strip_prefix("local:") {
			Some(token) => Self::Extracted(normalize_key(token)),
			None => Self::Number(normalize_key(rendered)),
		}
	}

	/// The raw text behind the key, without the `Display` prefix.
	pub fn raw(&self) -> String {
		match self {
			Self::Number(number) => number.clone(),
			Self::CanvasUser(id) => id.to_string(),
			Self::Extracted(token) => token.clone(),
		}
	}
}

/// Who a submission belongs to. The separate id fields are kept apart on purpose:
/// 学号, Canvas user id, SIS id and login id are four different things.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudentIdentity {
	pub key: StudentKey,
	#[serde(default)]
	pub student_number: Option<String>,
	#[serde(default)]
	pub canvas_user_id: Option<u64>,
	#[serde(default)]
	pub sis_user_id: Option<String>,
	#[serde(default)]
	pub login_id: Option<String>,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub sortable_name: Option<String>,
	#[serde(default)]
	pub email: Option<String>,
}

impl StudentIdentity {
	fn bare(key: StudentKey) -> Self {
		Self {
			key,
			student_number: None,
			canvas_user_id: None,
			sis_user_id: None,
			login_id: None,
			name: None,
			sortable_name: None,
			email: None,
		}
	}

	/// A student identified by a confirmed 学号.
	pub fn number(student_number: impl Into<String>) -> Self {
		let number = normalize_key(&student_number.into());
		Self {
			student_number: Some(number.clone()),
			..Self::bare(StudentKey::Number(number))
		}
	}

	/// A Canvas user with no SIS id to key on.
	pub fn canvas_user(canvas_user_id: u64) -> Self {
		Self {
			canvas_user_id: Some(canvas_user_id),
			..Self::bare(StudentKey::CanvasUser(canvas_user_id))
		}
	}

	/// A token extracted from a local filename, confirmed by nothing.
	pub fn extracted(token: impl Into<String>) -> Self {
		let token = normalize_key(&token.into());
		Self::bare(StudentKey::Extracted(token))
	}

	/// Promote an unconfirmed token to a confirmed 学号 once a roster vouches for it.
	pub fn confirm_number(&mut self) {
		if let StudentKey::Extracted(token) = &self.key {
			let number = token.clone();
			self.key = StudentKey::Number(number.clone());
			self.student_number = Some(number);
		}
	}
}

/// A file a student uploaded, as the source describes it. Becomes a [`StudentFile`] only
/// once the bytes are actually on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
	pub id: u64,
	pub filename: String,
	#[serde(default)]
	pub content_type: Option<String>,
	#[serde(default)]
	pub size: Option<u64>,
	#[serde(default)]
	pub url: Option<String>,
}

/// Source-reported state for one attempt. Canvas fills this; local input leaves it `None`
/// rather than fabricating `late: false`, which would be a claim rather than a default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
	pub workflow_state: String,
	#[serde(default)]
	pub late: bool,
	#[serde(default)]
	pub missing: bool,
	#[serde(default)]
	pub excused: bool,
}

/// Where a [`StudentFile`] came from, so every graded byte traces back to its origin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOrigin {
	/// Found directly in a scanned directory.
	#[default]
	Direct,
	/// Extracted from an archive. `entry` is the path *inside* the archive.
	Archive { archive: PathBuf, entry: String },
	/// Downloaded from a Canvas attachment. `entry` is the path *inside* the attachment
	/// when it was an archive that had to be expanded, and `None` when the attachment is
	/// the file itself. The archive's own path is recoverable from `attachment_id`, so a
	/// separate variant would only be a second way to say the same thing.
	Attachment {
		attempt: u32,
		attachment_id: u64,
		#[serde(default)]
		entry: Option<String>,
	},
}

/// A single runnable file belonging to a student's submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudentFile {
	pub path: PathBuf,
	pub language: String,
	#[serde(default)]
	pub origin: FileOrigin,
}

impl StudentFile {
	/// A file found directly on disk, with no archive or attachment behind it.
	pub fn direct(path: impl Into<PathBuf>, language: impl Into<String>) -> Self {
		Self {
			path: path.into(),
			language: language.into(),
			origin: FileOrigin::Direct,
		}
	}

	pub fn with_origin(mut self, origin: FileOrigin) -> Self {
		self.origin = origin;
		self
	}

	pub fn file_name(&self) -> String {
		self.path
			.file_name()
			.map(|n| n.to_string_lossy().into_owned())
			.unwrap_or_default()
	}
}

/// One submission attempt. Local input produces exactly one; Canvas may produce several.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionAttempt {
	pub attempt: u32,
	#[serde(default)]
	pub submitted_at: Option<String>,
	#[serde(default)]
	pub source_status: Option<SourceStatus>,
	/// What the source says was uploaded.
	#[serde(default)]
	pub attachments: Vec<Attachment>,
	/// What is actually on disk and runnable.
	#[serde(default)]
	pub files: Vec<StudentFile>,
}

impl SubmissionAttempt {
	pub fn new(attempt: u32) -> Self {
		Self {
			attempt,
			submitted_at: None,
			source_status: None,
			attachments: Vec::new(),
			files: Vec::new(),
		}
	}

	pub fn with_files(mut self, files: Vec<StudentFile>) -> Self {
		self.files = files;
		self
	}
}

/// How the material was delivered. Orthogonal to [`RosterMatch`] — keeping the two axes
/// apart is what stops `(ReceivedUnmatched, Matched)` from being representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
	/// On the roster, nothing received.
	NotSubmitted,
	/// Material received, but nothing runnable in it.
	SubmittedEmpty,
	/// At least one file in a recognised language.
	Executable,
}

/// Whether the submitter could be tied to a roster entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterMatch {
	/// Index into [`Roster::entries`].
	Matched(usize),
	NotInRoster,
	/// No roster was supplied at all, so membership is simply unknown.
	NoRoster,
}

/// The four outcomes the ticket requires be distinguishable. Computed from the two stored
/// axes rather than stored, so the two can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionOutcome {
	NotSubmitted,
	SubmittedEmpty,
	ReceivedUnmatched,
	Executable,
}

/// Everything known about one student's participation in one assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudentSubmission {
	pub identity: StudentIdentity,
	pub roster_match: RosterMatch,
	pub state: SubmissionState,
	pub attempts: Vec<SubmissionAttempt>,
	/// Index into `attempts`, chosen by [`AttemptPolicy`]. `None` iff nothing was received.
	pub selected: Option<usize>,
	/// The source's status for the submission *record*, as opposed to any one attempt.
	///
	/// `excused` and `missing` describe the record: a teacher excuses a student, not an
	/// attempt, and the excusal commonly lands on a row with no attempt at all — which is
	/// the canonical 免交 and the case an attempt-only home cannot represent. Only Canvas
	/// fills this; local input has no such notion.
	#[serde(default)]
	pub record_status: Option<SourceStatus>,
}

impl StudentSubmission {
	/// A roster student from whom nothing arrived.
	///
	/// Always `Matched`, never `NotInRoster`: a non-submitter only exists because a roster
	/// vouches for them.
	pub fn not_submitted(
		identity: StudentIdentity,
		roster_index: usize,
		record_status: Option<SourceStatus>,
	) -> Self {
		Self {
			identity,
			roster_match: RosterMatch::Matched(roster_index),
			state: SubmissionState::NotSubmitted,
			attempts: Vec::new(),
			selected: None,
			record_status,
		}
	}

	/// The status of the attempt actually being graded — per-attempt provenance.
	///
	/// Use [`Self::record_status`] for `excused` / `missing`: those describe the submission
	/// record, and reading them off an attempt reports an excusal applied after the
	/// selected attempt as absent.
	pub fn attempt_status(&self) -> Option<&SourceStatus> {
		self.selected
			.and_then(|i| self.attempts.get(i))
			.and_then(|a| a.source_status.as_ref())
	}

	/// Whether the source says this student was excused. Authoritative regardless of which
	/// attempt is selected, and true for the 免交 case that carries no attempt at all.
	pub fn is_excused(&self) -> bool {
		self.record_status.as_ref().is_some_and(|s| s.excused)
	}

	pub fn with_record_status(mut self, status: Option<SourceStatus>) -> Self {
		self.record_status = status;
		self
	}

	/// A student with received material. `state` is derived from the selected attempt, so
	/// it can never contradict the files.
	pub fn received(
		identity: StudentIdentity,
		roster_match: RosterMatch,
		attempts: Vec<SubmissionAttempt>,
		policy: AttemptPolicy,
	) -> Self {
		debug_assert!(
			!attempts.is_empty(),
			"received() needs at least one attempt; a student with nothing to show for \
			 themselves is either not_submitted() or has one empty attempt"
		);
		let selected = select_attempt(&attempts, policy);
		let state = match selected.and_then(|i| attempts.get(i)) {
			Some(attempt) if !attempt.files.is_empty() => SubmissionState::Executable,
			Some(_) => SubmissionState::SubmittedEmpty,
			None => SubmissionState::NotSubmitted,
		};
		Self {
			identity,
			roster_match,
			state,
			attempts,
			selected,
			record_status: None,
		}
	}

	/// One student, one attempt, a list of Python files — the shape most callers and tests
	/// want when they already know who the files belong to.
	pub fn from_files(
		student_number: impl Into<String>,
		paths: &[impl AsRef<std::path::Path>],
	) -> Self {
		let files: Vec<StudentFile> = paths
			.iter()
			.map(|p| StudentFile::direct(p.as_ref().to_path_buf(), "python"))
			.collect();
		Self::received(
			StudentIdentity::number(student_number),
			RosterMatch::NoRoster,
			vec![SubmissionAttempt::new(1).with_files(files)],
			AttemptPolicy::Latest,
		)
	}

	pub fn key(&self) -> &StudentKey {
		&self.identity.key
	}

	pub fn selected_attempt(&self) -> Option<&SubmissionAttempt> {
		self.selected.and_then(|i| self.attempts.get(i))
	}

	/// The runnable files of the selected attempt. Reading through `selected` means the
	/// files can never belong to a different attempt than the one that was chosen.
	pub fn files(&self) -> &[StudentFile] {
		self.selected_attempt()
			.map(|a| a.files.as_slice())
			.unwrap_or(&[])
	}

	pub fn outcome(&self) -> SubmissionOutcome {
		if self.state != SubmissionState::NotSubmitted
			&& matches!(self.roster_match, RosterMatch::NotInRoster)
		{
			return SubmissionOutcome::ReceivedUnmatched;
		}
		match self.state {
			SubmissionState::NotSubmitted => SubmissionOutcome::NotSubmitted,
			SubmissionState::SubmittedEmpty => SubmissionOutcome::SubmittedEmpty,
			SubmissionState::Executable => SubmissionOutcome::Executable,
		}
	}

	/// Names of the artifacts attributed to this student, whichever source they came from:
	/// attachment names when the source reported any, on-disk basenames otherwise.
	pub fn artifact_names(&self) -> Vec<String> {
		let Some(attempt) = self.selected_attempt() else {
			return Vec::new();
		};
		let mut names: Vec<String> = if attempt.attachments.is_empty() {
			attempt.files.iter().map(StudentFile::file_name).collect()
		} else {
			attempt
				.attachments
				.iter()
				.map(|a| a.filename.clone())
				.collect()
		};
		names.sort();
		names
	}

	/// Total ordering key: independent of `read_dir` order, and total even for duplicates.
	fn sort_key(&self) -> (StudentKey, Option<u64>, PathBuf) {
		(
			self.identity.key.clone(),
			self.identity.canvas_user_id,
			self.files()
				.first()
				.map(|f| f.path.clone())
				.unwrap_or_default(),
		)
	}
}

fn select_attempt(attempts: &[SubmissionAttempt], policy: AttemptPolicy) -> Option<usize> {
	if attempts.is_empty() {
		return None;
	}
	let pick = match policy {
		// Highest attempt number wins. `submitted_at` is only a tie-break: the workspace
		// has no date library, so comparing timestamps means a lexicographic string
		// compare — correct for uniform UTC and silently wrong otherwise.
		AttemptPolicy::Latest => attempts.iter().enumerate().max_by(|(_, a), (_, b)| {
			a.attempt
				.cmp(&b.attempt)
				.then(a.submitted_at.cmp(&b.submitted_at))
		}),
		AttemptPolicy::Earliest => attempts.iter().enumerate().min_by(|(_, a), (_, b)| {
			a.attempt
				.cmp(&b.attempt)
				.then(a.submitted_at.cmp(&b.submitted_at))
		}),
	};
	pick.map(|(i, _)| i)
}

/// Material that arrived but could not be attributed to any student.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnmatchedArtifact {
	pub path: PathBuf,
	pub reason: UnmatchedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedReason {
	/// No student key could be extracted from the filename.
	NoStudentKey,
	/// A keyless file in a format no backend runs.
	UnsupportedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
	Info,
	Warning,
	Error,
}

/// Anomalies found while building the input. Data, not a pre-rendered string: the text is
/// derived from the fields so the two cannot drift.
#[derive(
	Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, thiserror::Error,
)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
	#[error("roster lists '{key}' in {count} identical rows; merged into one")]
	DuplicateRosterEntry { key: String, count: usize },
	#[error("roster rows disagree about who '{key}' is: {detail}")]
	ConflictingRosterEntry { key: String, detail: String },
	#[error("student numbers differ only by zero padding: {keys:?}; kept distinct")]
	SuspectedZeroPaddedVariant { keys: Vec<String> },
	#[error("Canvas user {canvas_user_id} has no sis_user_id; keyed by Canvas id")]
	MissingStudentNumber { canvas_user_id: u64 },
	#[error("'{key}' submitted but is not on the roster")]
	NotOnRoster { key: String },
	#[error("roster row unusable ({reason}); the student it names is not in the input")]
	UnusableRosterRow { reason: String },
	#[error("ignored '{path}' for '{key}': not a supported submission file")]
	IgnoredFile { key: String, path: PathBuf },
	#[error(
		"'{key}' submitted '{path}': {format} archives cannot be opened, so its contents \
		 cannot be graded — .zip, .7z, .tar and .tar.gz can"
	)]
	UnsupportedArchive {
		key: String,
		path: PathBuf,
		format: String,
	},
	#[error("archive '{archive}' expanded to nothing")]
	ArchiveEmpty { archive: PathBuf },
	#[error("archive '{archive}' entry '{entry}' collides with an already extracted name")]
	ArchiveNameCollision { archive: PathBuf, entry: String },
	#[error("archive '{archive}' could not be read: {reason}")]
	ArchiveUnreadable { archive: PathBuf, reason: String },
	#[error("archive '{archive}' entry '{entry}' skipped: {reason}")]
	ArchiveEntrySkipped {
		archive: PathBuf,
		entry: String,
		reason: String,
	},
	#[error("attachment {attachment_id} ('{filename}') for '{key}' was never downloaded")]
	PendingDownload {
		key: String,
		attachment_id: u64,
		filename: String,
	},
	#[error(
		"attachment {attachment_id} ('{filename}') for '{key}' could not be downloaded: {reason}"
	)]
	AttachmentUnavailable {
		key: String,
		attachment_id: u64,
		filename: String,
		reason: String,
	},
	#[error("'{key}' submitted via '{submission_type}', which cannot be graded automatically")]
	UnsupportedSubmissionType {
		key: String,
		submission_type: String,
	},
	#[error("'{key}' submitted a text entry with no gradeable file")]
	TextEntryOnly { key: String },
	#[error(
		"Canvas reported more than one submission row for user {canvas_user_id}; kept the \
		 richer one (attempt {kept:?})"
	)]
	DuplicateSubmissionRow {
		canvas_user_id: u64,
		kept: Option<u32>,
	},
	#[error("'{key}' is on the supplied roster but is not enrolled in the Canvas course")]
	NotEnrolled { key: String },
	#[error("could not read an entry of '{dir}': {reason}")]
	UnreadableDirEntry { dir: PathBuf, reason: String },
}

/// Where in the source an anomaly was found. `sheet`/`row` are for the spreadsheet
/// importer (P-672); the local and Canvas adapters only set `file`.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceLocation {
	#[serde(default)]
	pub file: Option<PathBuf>,
	#[serde(default)]
	pub sheet: Option<String>,
	#[serde(default)]
	pub row: Option<usize>,
}

impl SourceLocation {
	pub fn file(path: impl Into<PathBuf>) -> Self {
		Self {
			file: Some(path.into()),
			..Self::default()
		}
	}

	pub fn row(path: impl Into<PathBuf>, row: usize) -> Self {
		Self {
			file: Some(path.into()),
			sheet: None,
			row: Some(row),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InputDiagnostic {
	pub severity: DiagnosticSeverity,
	pub kind: DiagnosticKind,
	#[serde(default)]
	pub location: Option<SourceLocation>,
}

impl InputDiagnostic {
	pub fn warning(kind: DiagnosticKind) -> Self {
		Self {
			severity: DiagnosticSeverity::Warning,
			kind,
			location: None,
		}
	}

	pub fn info(kind: DiagnosticKind) -> Self {
		Self {
			severity: DiagnosticSeverity::Info,
			kind,
			location: None,
		}
	}

	/// Something the run cannot sensibly continue past. The CLI refuses to grade while any
	/// of these is present rather than producing results nobody should trust.
	pub fn error(kind: DiagnosticKind) -> Self {
		Self {
			severity: DiagnosticSeverity::Error,
			kind,
			location: None,
		}
	}

	pub fn at(mut self, location: SourceLocation) -> Self {
		self.location = Some(location);
		self
	}
}

impl fmt::Display for InputDiagnostic {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.kind)
	}
}

/// Which entry point produced an input. Recorded once, on the input as a whole — a
/// per-student copy would let a `Local` input hold Canvas-sourced students.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSource {
	Canvas {
		#[serde(default)]
		course_id: Option<u64>,
		#[serde(default)]
		assignment_id: Option<u64>,
	},
	Local {
		scanned_dirs: Vec<PathBuf>,
		#[serde(default)]
		roster_path: Option<PathBuf>,
	},
}

/// One thing a student is marked on.
///
/// `id` is the test spec's `[meta] name`, which is what `TestResult.item_id` carries — so
/// the assignment's declared items and the results reference the same identity rather than
/// two parallel notions of "a question". Scores, weights and how evidence inside an item
/// aggregates are P-677's; this is identity only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GradingItem {
	pub id: String,
	#[serde(default)]
	pub title: Option<String>,
}

impl GradingItem {
	pub fn new(id: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			title: None,
		}
	}

	/// What to show a human: the title when the teacher gave one, else the id.
	pub fn label(&self) -> &str {
		self.title.as_deref().unwrap_or(&self.id)
	}
}

/// Assignment identity. Course id and assignment id are stored apart from the name, and
/// apart from any student identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
	pub name: String,
	#[serde(default)]
	pub canvas_course_id: Option<u64>,
	#[serde(default)]
	pub canvas_assignment_id: Option<u64>,
	/// The items this assignment is marked on, in declaration order.
	#[serde(default)]
	pub items: Vec<GradingItem>,
}

impl Assignment {
	pub fn named(name: impl Into<String>) -> Self {
		Self {
			name: name.into(),
			..Self::default()
		}
	}

	pub fn with_items(mut self, items: Vec<GradingItem>) -> Self {
		self.items = items;
		self
	}

	pub fn item(&self, id: &str) -> Option<&GradingItem> {
		self.items.iter().find(|item| item.id == id)
	}
}

/// The unified contract. Whatever the entry point, this is what downstream reads.
///
/// Binding files and functions to items is P-673; per-item scoring is P-677.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentInput {
	pub assignment: Assignment,
	pub source: InputSource,
	#[serde(default)]
	pub roster: Option<Roster>,
	pub students: Vec<StudentSubmission>,
	/// The rule that chose every `selected` index. Without it the record says which attempt
	/// was graded but not why, and cannot be checked against the results produced from it.
	#[serde(default)]
	pub attempt_policy: AttemptPolicy,
	#[serde(default)]
	pub unmatched: Vec<UnmatchedArtifact>,
	#[serde(default)]
	pub diagnostics: Vec<InputDiagnostic>,
}

/// The cross-source comparison shape. Canvas carries user ids and attempt timestamps that
/// local input simply does not have, so equivalence is asserted on this projection rather
/// than on the whole struct.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProjectedStudent {
	pub key: String,
	pub outcome: SubmissionOutcome,
	pub artifacts: Vec<String>,
}

impl AssignmentInput {
	pub fn new(assignment: Assignment, source: InputSource) -> Self {
		Self {
			assignment,
			source,
			roster: None,
			students: Vec::new(),
			attempt_policy: AttemptPolicy::default(),
			unmatched: Vec::new(),
			diagnostics: Vec::new(),
		}
	}

	/// Impose a total order on every vector. `read_dir` order is not stable across
	/// filesystems, so without this any assertion on the output is flaky in CI.
	pub fn sorted(mut self) -> Self {
		self.students.sort_by_key(StudentSubmission::sort_key);
		self.unmatched.sort();
		self.diagnostics
			.sort_by(|a, b| (&a.location, &a.kind).cmp(&(&b.location, &b.kind)));
		self
	}

	pub fn student_count(&self) -> usize {
		self.students.len()
	}

	pub fn with_outcome(
		&self,
		outcome: SubmissionOutcome,
	) -> impl Iterator<Item = &StudentSubmission> {
		self.students.iter().filter(move |s| s.outcome() == outcome)
	}

	/// Distinct languages across every selected attempt.
	pub fn languages(&self) -> Vec<String> {
		let mut langs: Vec<String> = self
			.students
			.iter()
			.flat_map(|s| s.files())
			.map(|f| f.language.clone())
			.collect();
		langs.sort();
		langs.dedup();
		langs
	}

	/// The cross-source comparison shape.
	///
	/// Keyed on [`StudentKey::raw`], not its `Display` form: whether a given student number
	/// counts as *confirmed* is a property of the source — Canvas vouches for its own SIS
	/// ids, a local filename token vouches for nothing — so comparing the prefixed form
	/// would report a difference in confidence as a difference in identity.
	pub fn projection(&self) -> Vec<ProjectedStudent> {
		let mut projected: Vec<ProjectedStudent> = self
			.students
			.iter()
			.map(|s| ProjectedStudent {
				key: s.identity.key.raw(),
				outcome: s.outcome(),
				artifacts: s.artifact_names(),
			})
			.collect();
		projected.sort();
		projected
	}

	pub fn diagnostics_of(
		&self,
		severity: DiagnosticSeverity,
	) -> impl Iterator<Item = &InputDiagnostic> {
		self.diagnostics
			.iter()
			.filter(move |d| d.severity == severity)
	}

	/// Anything that makes the input untrustworthy to grade from.
	pub fn errors(&self) -> impl Iterator<Item = &InputDiagnostic> {
		self.diagnostics_of(DiagnosticSeverity::Error)
	}

	/// One `SuspectedZeroPaddedVariant` per group of keys that differ only by zero
	/// padding. Keys are never merged — this only flags that an upstream export may have
	/// stripped the padding.
	pub fn detect_zero_padded_variants(&self) -> Vec<InputDiagnostic> {
		let mut buckets: std::collections::BTreeMap<String, Vec<String>> = Default::default();
		for student in &self.students {
			// Only keys that denote a student number are comparable. A Canvas id is a
			// separate namespace, so `CanvasUser(123)` beside `Number("00123")` is not a
			// padding variant — warning about it would be a false positive.
			let raw = match &student.identity.key {
				StudentKey::Number(number) => number.clone(),
				StudentKey::Extracted(token) => token.clone(),
				StudentKey::CanvasUser(_) => continue,
			};
			buckets
				.entry(zero_stripped(&raw).to_string())
				.or_default()
				.push(raw);
		}
		buckets
			.into_values()
			.filter_map(|mut keys| {
				keys.sort();
				keys.dedup();
				(keys.len() > 1).then(|| {
					InputDiagnostic::warning(DiagnosticKind::SuspectedZeroPaddedVariant { keys })
				})
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn executable_attempt(n: u32, file: &str) -> SubmissionAttempt {
		SubmissionAttempt::new(n).with_files(vec![StudentFile::direct(file, "python")])
	}

	#[test]
	fn test_key_display_cannot_be_confused_with_a_number() {
		assert_eq!(StudentKey::Number("012345".into()).to_string(), "012345");
		assert_eq!(StudentKey::CanvasUser(12345).to_string(), "canvas:12345");
		assert_eq!(
			StudentKey::Extracted("notes".into()).to_string(),
			"local:notes"
		);
	}

	#[test]
	fn test_leading_zeros_are_never_stripped_from_a_key() {
		let identity = StudentIdentity::number("0024010003");
		assert_eq!(identity.key, StudentKey::Number("0024010003".into()));
		assert_eq!(identity.student_number.as_deref(), Some("0024010003"));
		assert_ne!(identity.key, StudentIdentity::number("24010003").key);
	}

	#[test]
	fn test_normalize_key_trims_and_strips_bom_only() {
		assert_eq!(normalize_key("\u{feff} 2024010001 "), "2024010001");
		assert_eq!(normalize_key("0024010003"), "0024010003");
	}

	#[test]
	fn test_not_submitted_is_always_roster_matched() {
		let s = StudentSubmission::not_submitted(StudentIdentity::number("2024010004"), 3, None);
		assert_eq!(s.state, SubmissionState::NotSubmitted);
		assert_eq!(s.roster_match, RosterMatch::Matched(3));
		assert_eq!(s.outcome(), SubmissionOutcome::NotSubmitted);
		assert!(s.files().is_empty());
	}

	#[test]
	fn test_outcome_separates_all_four_cases() {
		let executable = StudentSubmission::received(
			StudentIdentity::number("1"),
			RosterMatch::Matched(0),
			vec![executable_attempt(1, "1_lab.py")],
			AttemptPolicy::Latest,
		);
		assert_eq!(executable.outcome(), SubmissionOutcome::Executable);

		let empty = StudentSubmission::received(
			StudentIdentity::number("2"),
			RosterMatch::Matched(1),
			vec![SubmissionAttempt::new(1)],
			AttemptPolicy::Latest,
		);
		assert_eq!(empty.outcome(), SubmissionOutcome::SubmittedEmpty);

		// Not on the roster wins over the delivery axis, whether or not the files run.
		let unmatched_runnable = StudentSubmission::received(
			StudentIdentity::extracted("9999"),
			RosterMatch::NotInRoster,
			vec![executable_attempt(1, "9999_lab.py")],
			AttemptPolicy::Latest,
		);
		assert_eq!(
			unmatched_runnable.outcome(),
			SubmissionOutcome::ReceivedUnmatched
		);

		let unmatched_empty = StudentSubmission::received(
			StudentIdentity::extracted("8888"),
			RosterMatch::NotInRoster,
			vec![SubmissionAttempt::new(1)],
			AttemptPolicy::Latest,
		);
		assert_eq!(
			unmatched_empty.outcome(),
			SubmissionOutcome::ReceivedUnmatched
		);

		let absent = StudentSubmission::not_submitted(StudentIdentity::number("4"), 0, None);
		assert_eq!(absent.outcome(), SubmissionOutcome::NotSubmitted);
	}

	#[test]
	fn test_latest_attempt_wins_and_files_follow_the_selection() {
		let student = StudentSubmission::received(
			StudentIdentity::number("2024010002"),
			RosterMatch::Matched(0),
			vec![
				executable_attempt(1, "first.py"),
				executable_attempt(2, "second.py"),
			],
			AttemptPolicy::Latest,
		);
		assert_eq!(student.selected_attempt().unwrap().attempt, 2);
		assert_eq!(student.files()[0].file_name(), "second.py");

		// Order in the payload must not decide the selection.
		let reversed = StudentSubmission::received(
			StudentIdentity::number("2024010002"),
			RosterMatch::Matched(0),
			vec![
				executable_attempt(2, "second.py"),
				executable_attempt(1, "first.py"),
			],
			AttemptPolicy::Latest,
		);
		assert_eq!(reversed.selected_attempt().unwrap().attempt, 2);
		assert_eq!(reversed.files()[0].file_name(), "second.py");
	}

	#[test]
	fn test_earliest_attempt_policy() {
		let student = StudentSubmission::received(
			StudentIdentity::number("2024010002"),
			RosterMatch::Matched(0),
			vec![
				executable_attempt(2, "second.py"),
				executable_attempt(1, "first.py"),
			],
			AttemptPolicy::Earliest,
		);
		assert_eq!(student.selected_attempt().unwrap().attempt, 1);
	}

	#[test]
	fn test_artifact_names_prefer_attachments_then_fall_back_to_files() {
		let mut attempt = executable_attempt(1, "/tmp/2024010001_lab1.py");
		assert_eq!(
			StudentSubmission::received(
				StudentIdentity::number("2024010001"),
				RosterMatch::Matched(0),
				vec![attempt.clone()],
				AttemptPolicy::Latest,
			)
			.artifact_names(),
			vec!["2024010001_lab1.py"]
		);

		attempt.attachments.push(Attachment {
			id: 1,
			filename: "lab1.py".into(),
			content_type: None,
			size: None,
			url: None,
		});
		assert_eq!(
			StudentSubmission::received(
				StudentIdentity::number("2024010001"),
				RosterMatch::Matched(0),
				vec![attempt],
				AttemptPolicy::Latest,
			)
			.artifact_names(),
			vec!["lab1.py"]
		);
	}

	#[test]
	fn test_zero_padded_variants_are_flagged_but_never_merged() {
		let mut input = AssignmentInput::new(
			Assignment::named("hw1"),
			InputSource::Local {
				scanned_dirs: vec![],
				roster_path: None,
			},
		);
		input.students = vec![
			StudentSubmission::not_submitted(StudentIdentity::number("0024010003"), 0, None),
			StudentSubmission::not_submitted(StudentIdentity::number("24010003"), 1, None),
			StudentSubmission::not_submitted(StudentIdentity::number("2024010001"), 2, None),
		];

		let found = input.detect_zero_padded_variants();
		assert_eq!(found.len(), 1);
		assert!(matches!(
			&found[0].kind,
			DiagnosticKind::SuspectedZeroPaddedVariant { keys } if keys == &["0024010003", "24010003"]
		));
		// Both survive as separate students.
		assert_eq!(input.students.len(), 3);
	}

	#[test]
	fn test_a_canvas_id_is_not_a_zero_padding_variant_of_a_student_number() {
		let mut input = AssignmentInput::new(
			Assignment::named("hw1"),
			InputSource::Canvas {
				course_id: None,
				assignment_id: None,
			},
		);
		input.students = vec![
			StudentSubmission::not_submitted(StudentIdentity::number("00123"), 0, None),
			StudentSubmission::not_submitted(StudentIdentity::canvas_user(123), 1, None),
		];

		// Separate namespaces — comparing their padding would be a false positive.
		assert!(input.detect_zero_padded_variants().is_empty());
	}

	#[test]
	fn test_reserved_prefixes_are_recognised() {
		assert!(is_reserved_key("local:alice"));
		assert!(is_reserved_key("canvas:5"));
		assert!(!is_reserved_key("2024010001"));
		assert!(!is_reserved_key("localhost"));
	}

	#[test]
	fn test_confirm_number_promotes_an_extracted_token() {
		let mut identity = StudentIdentity::extracted("2024010001");
		assert!(identity.student_number.is_none());
		identity.confirm_number();
		assert_eq!(identity.key, StudentKey::Number("2024010001".into()));
		assert_eq!(identity.student_number.as_deref(), Some("2024010001"));
	}

	#[test]
	fn test_grading_item_identity_is_the_spec_name() {
		let assignment = Assignment::named("hw1").with_items(vec![
			GradingItem::new("find_larger_number"),
			GradingItem {
				id: "sum_pair".to_string(),
				title: Some("第二题 求和".to_string()),
			},
		]);

		// A result references an item by the same id a test spec's [meta] name carries.
		let result = crate::models::TestResult {
			item_id: "sum_pair".to_string(),
			cases: vec![],
		};
		let item = assignment.item(&result.item_id).expect("declared item");
		assert_eq!(item.label(), "第二题 求和");
		// Without a title a human still gets something meaningful.
		assert_eq!(
			assignment.item("find_larger_number").unwrap().label(),
			"find_larger_number"
		);
		assert!(assignment.item("nope").is_none());
	}

	#[test]
	fn test_results_written_as_spec_name_still_load() {
		// The field was called `spec_name` before items were modelled.
		let legacy = r#"{"spec_name": "find_larger_number", "cases": []}"#;
		let result: crate::models::TestResult = serde_json::from_str(legacy).unwrap();
		assert_eq!(result.item_id, "find_larger_number");
	}

	#[test]
	fn test_new_enums_serialise_snake_case() {
		let json = serde_json::to_string(&SubmissionOutcome::ReceivedUnmatched).unwrap();
		assert_eq!(json, "\"received_unmatched\"");
		let json = serde_json::to_string(&FileOrigin::Direct).unwrap();
		assert_eq!(json, "\"direct\"");
	}
}
