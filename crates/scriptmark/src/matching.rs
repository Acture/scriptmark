//! Teacher rules and auditable decisions. A fuzzy candidate is a suggestion, never a
//! substitute for a student's declared file or function.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::models::{FileOrigin, StudentFile, StudentSubmission, Target, TestSpec, normalize_key};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
	pub students: Vec<StudentRule>,
	pub owners: Vec<OwnerOverride>,
	pub items: Vec<ItemRule>,
	pub overrides: Vec<ItemOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StudentRule {
	/// Zero-based component of the directory path relative to the scanned root.
	Directory { level: usize },
	/// A filename template with one `{student}` capture; `*` and `?` are wildcards.
	Pattern { pattern: String },
	/// Matches the relative path; must capture `(?P<student>...)`.
	Regex { regex: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerOverride {
	/// Relative path within a scanned root, or an absolute path.
	pub path: PathBuf,
	pub student: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRule {
	pub id: String,
	/// Alternative filename/relative path patterns. All matching files are candidates.
	#[serde(default)]
	pub files: Vec<String>,
	/// Requested function -> acceptable aliases. The exact requested name wins.
	#[serde(default)]
	pub functions: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemOverride {
	/// A rendered student key (`local:alice`, `canvas:123`, or a confirmed number).
	pub student: String,
	pub item: String,
	/// Exact submitted path or a path suffix. More than one hit remains ambiguous.
	#[serde(default)]
	pub file: Option<PathBuf>,
	#[serde(default)]
	pub functions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
	Matched,
	Missing,
	Ambiguous,
	Review,
	/// Static inspection cannot see a runtime export (or the module has a syntax error).
	Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Candidate {
	pub value: String,
	pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Decision {
	pub state: State,
	pub selected: Option<String>,
	pub candidates: Vec<Candidate>,
}

impl Decision {
	/// Retain inconsistent runtime lookups as review evidence, in unit declaration order.
	pub fn merge(&mut self, other: Self) {
		if *self == other {
			return;
		}
		let candidates = self
			.candidates
			.iter()
			.cloned()
			.chain(other.candidates)
			.collect();
		*self = Self::from_candidates(candidates, true);
	}
	pub fn exact(value: String, rule: &str) -> Self {
		Self::from_candidates(
			vec![Candidate {
				value,
				rules: vec![rule.into()],
			}],
			false,
		)
	}

	fn from_candidates(candidates: Vec<Candidate>, fuzzy: bool) -> Self {
		let mut grouped: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
		for candidate in candidates {
			grouped
				.entry(candidate.value)
				.or_default()
				.extend(candidate.rules);
		}
		let candidates: Vec<Candidate> = grouped
			.into_iter()
			.map(|(value, rules)| Candidate {
				value,
				rules: rules.into_iter().collect(),
			})
			.collect();
		let state = match candidates.len() {
			0 => State::Missing,
			_ if fuzzy => State::Review,
			1 => State::Matched,
			_ => State::Ambiguous,
		};
		Self {
			selected: (state == State::Matched).then(|| candidates[0].value.clone()),
			state,
			candidates,
		}
	}
}

fn pattern_regex(pattern: &str, capture: bool) -> Result<Regex> {
	let mut expression = String::from("^");
	let mut rest = pattern;
	while !rest.is_empty() {
		if capture && rest.starts_with("{student}") {
			expression.push_str("(?P<student>[^/]+?)");
			rest = &rest[9..];
			continue;
		}
		let ch = rest.chars().next().expect("nonempty pattern");
		match ch {
			'*' => expression.push_str(".*"),
			'?' => expression.push('.'),
			_ => expression.push_str(&regex::escape(&ch.to_string())),
		}
		rest = &rest[ch.len_utf8()..];
	}
	expression.push('$');
	Ok(Regex::new(&expression)?)
}

impl Config {
	pub fn validate_owners(&self) -> Result<()> {
		for rule in &self.students {
			match rule {
				StudentRule::Pattern { pattern } => {
					if pattern.matches("{student}").count() != 1 {
						bail!("student pattern needs exactly one {{student}}: {pattern}");
					}
					pattern_regex(pattern, true)?;
				}
				StudentRule::Regex { regex } => {
					let compiled = Regex::new(regex)?;
					if !compiled.capture_names().any(|name| name == Some("student")) {
						bail!("student regex needs a named student capture: {regex}");
					}
				}
				StudentRule::Directory { .. } => {}
			}
		}
		for owner in &self.owners {
			if owner.path.as_os_str().is_empty() || normalize_key(&owner.student).is_empty() {
				bail!("an owner override needs a path and student");
			}
		}
		Ok(())
	}

	pub fn validate(&self, specs: &[TestSpec]) -> Result<()> {
		self.validate_owners()?;
		let mut ids = BTreeSet::new();
		for item in &self.items {
			if !ids.insert(&item.id) {
				bail!("matching item '{}' is declared twice", item.id);
			}
			let spec = specs
				.iter()
				.find(|s| s.meta.name == item.id)
				.ok_or_else(|| anyhow::anyhow!("matching item '{}' has no test spec", item.id))?;
			for pattern in &item.files {
				if pattern.is_empty() {
					bail!("empty file pattern for '{}'", item.id);
				}
				pattern_regex(pattern, false)?;
			}
			validate_functions(item.functions.keys(), spec)?;
			if item
				.functions
				.values()
				.flatten()
				.any(|name| name.is_empty())
			{
				bail!("empty function alias for '{}'", item.id);
			}
		}
		let mut overrides = BTreeSet::new();
		for entry in &self.overrides {
			if entry.student.is_empty() || !overrides.insert((&entry.student, &entry.item)) {
				bail!(
					"duplicate or empty student override for '{}' / '{}'",
					entry.student,
					entry.item
				);
			}
			let spec = specs
				.iter()
				.find(|s| s.meta.name == entry.item)
				.ok_or_else(|| {
					anyhow::anyhow!("override item '{}' has no test spec", entry.item)
				})?;
			validate_functions(entry.functions.keys(), spec)?;
			if entry.functions.values().any(|name| name.is_empty())
				|| entry
					.file
					.as_ref()
					.is_some_and(|path| path.as_os_str().is_empty())
			{
				bail!("empty file/function override for '{}'", entry.item);
			}
		}
		Ok(())
	}

	pub fn owner(
		&self,
		relative: &Path,
		original: &Path,
		default: Option<String>,
	) -> Result<Decision> {
		let candidates: Vec<Candidate> = self
			.owners
			.iter()
			.enumerate()
			.filter(|(_, entry)| entry.path == relative || entry.path == original)
			.map(|(i, entry)| Candidate {
				value: normalize_key(&entry.student),
				rules: vec![format!(
					"matching.owners[{i}]: {} -> {}",
					entry.path.display(),
					entry.student
				)],
			})
			.collect();
		if !candidates.is_empty() {
			return Ok(Decision::from_candidates(candidates, false));
		}
		let mut candidates = Vec::new();
		for (i, rule) in self.students.iter().enumerate() {
			let keys: Vec<String> = match rule {
				StudentRule::Directory { level } => relative
					.parent()
					.and_then(|p| p.components().nth(*level))
					.and_then(|c| c.as_os_str().to_str())
					.map(str::to_string)
					.into_iter()
					.collect(),
				StudentRule::Pattern { pattern } => pattern_regex(pattern, true)?
					.captures(&relative.file_name().unwrap_or_default().to_string_lossy())
					.and_then(|caps| caps.name("student").map(|m| m.as_str().to_string()))
					.into_iter()
					.collect(),
				StudentRule::Regex { regex } => Regex::new(regex)?
					.captures_iter(&relative.to_string_lossy())
					.filter_map(|caps| caps.name("student").map(|m| m.as_str().to_string()))
					.collect(),
			};
			for key in keys
				.into_iter()
				.map(|s| normalize_key(&s))
				.filter(|s| !s.is_empty())
			{
				candidates.push(Candidate {
					value: key,
					rules: vec![format!("matching.students[{i}]: {rule:?}")],
				});
			}
		}
		if !candidates.is_empty() {
			return Ok(Decision::from_candidates(candidates, false));
		}
		Ok(default
			.map(|key| Decision::exact(key, "source.filename_prefix"))
			.unwrap_or_else(|| Decision::from_candidates(Vec::new(), false)))
	}

	fn item_override(&self, student: &str, item: &str) -> Option<&ItemOverride> {
		self.overrides
			.iter()
			.find(|entry| entry.student == student && entry.item == item)
	}

	pub fn functions(&self, student: &str, item: &str) -> Functions {
		let aliases = self
			.items
			.iter()
			.find(|rule| rule.id == item)
			.map(|rule| rule.functions.clone())
			.unwrap_or_default();
		let overrides = self
			.item_override(student, item)
			.map(|entry| entry.functions.clone())
			.unwrap_or_default();
		Functions { aliases, overrides }
	}

	pub fn file(&self, student: &str, files: &[StudentFile], spec: &TestSpec) -> Decision {
		let language_files: Vec<&StudentFile> = files
			.iter()
			.filter(|f| f.language == spec.meta.language)
			.collect();
		if let Some(path) = self
			.item_override(student, &spec.meta.name)
			.and_then(|entry| entry.file.as_ref())
		{
			return file_candidates(
				&language_files,
				|file| file.path == *path || file.path.ends_with(path),
				&format!("matching.overrides.file = {}", path.display()),
				false,
			);
		}
		if let Some(rule) = self
			.items
			.iter()
			.find(|rule| rule.id == spec.meta.name && !rule.files.is_empty())
		{
			let mut candidates = Vec::new();
			for (i, pattern) in rule.files.iter().enumerate() {
				let expression = pattern_regex(pattern, false).expect("validated file pattern");
				for file in &language_files {
					let name = file.path.file_name().unwrap_or_default().to_string_lossy();
					let path = file.path.to_string_lossy();
					if expression.is_match(&name)
						|| path.split('/').enumerate().any(|(start, _)| {
							expression.is_match(
								&path.split('/').skip(start).collect::<Vec<_>>().join("/"),
							)
						}) {
						candidates.push(Candidate {
							value: file.path.to_string_lossy().into_owned(),
							rules: vec![format!(
								"matching.items[{}].files[{i}] = {pattern:?}",
								rule.id
							)],
						});
					}
				}
			}
			return Decision::from_candidates(candidates, false);
		}
		let pattern = &spec.meta.file;
		let exact = file_candidates(
			&language_files,
			|file| {
				file.path.ends_with(pattern) || file.file_name().ends_with(&format!("_{pattern}"))
			},
			"source.file_suffix",
			false,
		);
		if exact.state != State::Missing {
			return exact;
		}
		let stem = Path::new(pattern)
			.file_stem()
			.unwrap_or_default()
			.to_string_lossy();
		file_candidates(
			&language_files,
			|file| {
				if !stem.is_empty()
					&& file
						.file_name()
						.to_lowercase()
						.contains(&stem.to_lowercase())
				{
					return true;
				}
				let Ok(text) = std::fs::read_to_string(&file.path) else {
					return false;
				};
				requested_functions(spec).iter().any(|name| {
					Regex::new(&format!(
						r"(?m)^(?:async\s+)?(?:def|class)\s+{}\b",
						regex::escape(name)
					))
					.expect("escaped function")
					.is_match(&text)
				})
			},
			"suggestion.filename_or_function",
			true,
		)
	}
}

fn file_candidates(
	files: &[&StudentFile],
	predicate: impl Fn(&StudentFile) -> bool,
	rule: &str,
	fuzzy: bool,
) -> Decision {
	Decision::from_candidates(
		files
			.iter()
			.filter(|file| predicate(file))
			.map(|file| Candidate {
				value: file.path.to_string_lossy().into_owned(),
				rules: vec![rule.into()],
			})
			.collect(),
		fuzzy,
	)
}

fn validate_functions<'a>(names: impl Iterator<Item = &'a String>, spec: &TestSpec) -> Result<()> {
	let requested = requested_functions(spec);
	for name in names {
		if !requested.contains(name) {
			bail!("function '{name}' is not requested by '{}'", spec.meta.name);
		}
	}
	Ok(())
}

pub fn requested_functions(spec: &TestSpec) -> BTreeSet<String> {
	let mut names = BTreeSet::new();
	names.extend(spec.meta.function.iter().cloned());
	for case in spec
		.cases
		.iter()
		.chain(spec.scenarios.iter().flat_map(|s| s.steps.iter()))
	{
		if let Some(Target::Function { name }) = case.target(spec.meta.function.as_deref()) {
			names.insert(name);
		}
	}
	for setup in spec.scenarios.iter().flat_map(|s| s.setup.iter()) {
		if let Target::Function { name } = setup.target() {
			names.insert(name);
		}
	}
	names
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Functions {
	pub aliases: BTreeMap<String, Vec<String>>,
	pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemMatch {
	pub student: String,
	pub item: String,
	pub file: Decision,
	pub origin: Option<FileOrigin>,
	pub owner: Option<Decision>,
	#[serde(default)]
	pub functions: BTreeMap<String, Decision>,
}

pub fn item_match(config: &Config, student: &StudentSubmission, spec: &TestSpec) -> ItemMatch {
	let file = config.file(&student.key().to_string(), student.files(), spec);
	let selected = student
		.files()
		.iter()
		.find(|f| Some(f.path.to_string_lossy().as_ref()) == file.selected.as_deref());
	ItemMatch {
		student: student.key().to_string(),
		item: spec.meta.name.clone(),
		origin: selected.map(|f| f.origin.clone()),
		owner: selected.map(|f| {
			f.owner
				.clone()
				.unwrap_or_else(|| Decision::exact(student.key().to_string(), "source.identity"))
		}),
		file,
		functions: BTreeMap::new(),
	}
}

#[derive(Debug, Serialize)]
pub struct Preview {
	pub input: crate::models::AssignmentInput,
	pub rules: Config,
	pub items: Vec<ItemMatch>,
	/// Indices into `items`, plus `input.unmatched` / error diagnostics for ownership.
	pub pending: Vec<usize>,
}

/// Parse all selected Python files in one isolated interpreter, without importing them.
pub fn preview(
	input: crate::models::AssignmentInput,
	specs: &[TestSpec],
	config: &Config,
	python: &str,
) -> Result<Preview> {
	use std::io::Write;
	use std::process::{Command, Stdio};
	config.validate(specs)?;
	let mut items = Vec::new();
	let mut requests = Vec::new();
	let mut slots = Vec::new();
	for student in &input.students {
		for spec in specs {
			let item = item_match(config, student, spec);
			if let Some(path) = &item.file.selected {
				requests.push(serde_json::json!({
					"path": path, "requested": requested_functions(spec),
					"policy": config.functions(&item.student, &item.item),
				}));
				slots.push(items.len());
			}
			items.push(item);
		}
	}
	if !requests.is_empty() {
		let code = format!("{}\npreview_functions()", include_str!("matching.py"));
		let mut child = Command::new(python)
			.args(["-I", "-S", "-c", &code])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()?;
		child
			.stdin
			.take()
			.expect("piped stdin")
			.write_all(&serde_json::to_vec(&requests)?)?;
		let output = child.wait_with_output()?;
		if !output.status.success() {
			bail!(
				"function preview failed: {}",
				String::from_utf8_lossy(&output.stderr)
			);
		}
		let decisions: Vec<BTreeMap<String, Decision>> = serde_json::from_slice(&output.stdout)?;
		if decisions.len() != slots.len() {
			bail!("function preview returned a different item count");
		}
		for (slot, functions) in slots.into_iter().zip(decisions) {
			items[slot].functions = functions;
		}
	}
	let pending = items
		.iter()
		.enumerate()
		.filter(|(_, item)| {
			item.file.state != State::Matched
				|| item
					.functions
					.values()
					.any(|decision| decision.state != State::Matched)
		})
		.map(|(i, _)| i)
		.collect();
	Ok(Preview {
		input,
		rules: config.clone(),
		items,
		pending,
	})
}
