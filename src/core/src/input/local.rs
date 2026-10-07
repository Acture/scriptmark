//! Resolve teacher-owned local inputs before discovering or executing student files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::table;
use crate::matching::{self, OwnerOverride};
use crate::models::{DiagnosticKind, InputDiagnostic, SourceLocation, normalize_key};
use crate::roster::{Roster, RosterEntry};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
	pub submissions: Vec<PathBuf>,
	pub roster: Option<RosterConfig>,
	pub students: Vec<toml::Spanned<Student>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RosterConfig {
	pub path: Option<PathBuf>,
	pub sheet: Option<String>,
	pub header_row: Option<usize>,
	pub columns: Option<table::Columns>,
}

impl RosterConfig {
	pub fn options(&self) -> table::Options {
		table::Options {
			sheet: self.sheet.clone(),
			header_row: self.header_row,
			columns: self.columns.clone(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Student {
	pub student_id: String,
	pub name: Option<String>,
	pub canvas_user_id: Option<u64>,
	#[serde(default)]
	pub files: Vec<PathBuf>,
}

pub struct Resolved {
	pub paths: Vec<PathBuf>,
	pub roster: Option<Roster>,
	pub matching: matching::Config,
}

impl Config {
	pub fn is_empty(&self) -> bool {
		self.submissions.is_empty() && self.roster.is_none() && self.students.is_empty()
	}

	/// CLI paths use the working directory; configured paths use assignment.toml's directory.
	pub fn resolve(
		&self,
		assignment: Option<&Path>,
		submissions: &[PathBuf],
		roster_override: Option<&Path>,
		matching: &matching::Config,
	) -> Result<Resolved> {
		let resolve = |path: &Path| relative_to(assignment, path);
		if !self.students.is_empty() {
			if !self.submissions.is_empty()
				|| !submissions.is_empty()
				|| self.roster.is_some()
				|| roster_override.is_some()
			{
				bail!(
					"input.students is an explicit roster and file list; do not combine it with submission directories or a roster table"
				);
			}
			let config_path = assignment.context("input.students requires an assignment.toml")?;
			let source = std::fs::read_to_string(config_path)?;
			let mut entries = Vec::new();
			let mut diagnostics = Vec::new();
			let mut paths = Vec::new();
			let mut matching = matching.clone();
			for owner in &mut matching.owners {
				owner.path = resolve(&owner.path)?;
			}
			let mut owners: BTreeMap<PathBuf, String> = BTreeMap::new();
			for entry in &self.students {
				let row = source
					.get(..entry.span().start)
					.context("invalid source location in input.students")?
					.bytes()
					.filter(|b| *b == b'\n')
					.count() + 1;
				let location = SourceLocation::row(config_path, row);
				let student = entry.get_ref();
				let id = normalize_key(&student.student_id);
				if let Err(error) = table::valid_student_id(&id) {
					diagnostics.push(
						InputDiagnostic::error(DiagnosticKind::UnusableRosterRow {
							reason: error.to_string(),
						})
						.at(location),
					);
					continue;
				}
				let mut roster_entry = RosterEntry::new(
					&id,
					student
						.name
						.as_ref()
						.map(|name| name.trim().to_owned())
						.filter(|name| !name.is_empty()),
				);
				roster_entry.canvas_user_id = student.canvas_user_id;
				roster_entry.location = Some(location.clone());
				entries.push(roster_entry);
				if student.canvas_user_id == Some(0) {
					diagnostics.push(
						InputDiagnostic::error(DiagnosticKind::UnusableRosterRow {
							reason: "canvas_user_id must be positive".into(),
						})
						.at(location.clone()),
					);
				}
				for file in &student.files {
					let result = (|| -> Result<PathBuf> {
						let path = resolve(file)?;
						if !std::fs::metadata(&path)
							.with_context(|| format!("cannot read {}", path.display()))?
							.is_file()
						{
							bail!(
								"expected a file or archive; list individual files in input.students.files"
							);
						}
						let canonical = path.canonicalize()?;
						if let Some(other) = owners.get(&canonical) {
							bail!("file is listed more than once (first assigned to '{other}')");
						}
						owners.insert(canonical, id.clone());
						Ok(path)
					})();
					match result {
						Ok(path) => {
							matching.owners.push(OwnerOverride {
								path: path.clone(),
								student: id.clone(),
							});
							paths.push(path);
						}
						Err(error) => diagnostics.push(
							InputDiagnostic::error(DiagnosticKind::InvalidSubmissionPath {
								path: file.clone(),
								student: id.clone(),
								reason: format!("{error:#}"),
							})
							.at(location.clone()),
						),
					}
				}
			}
			return Ok(Resolved {
				paths,
				roster: Some(Roster::with_diagnostics(entries, diagnostics)),
				matching,
			});
		}
		let paths = if submissions.is_empty() {
			self.submissions
				.iter()
				.map(|path| {
					let resolved = resolve(path)?;
					std::fs::metadata(&resolved).with_context(|| {
						format!(
							"{}: cannot read input.submissions entry '{}'",
							assignment.unwrap_or(Path::new("assignment.toml")).display(),
							path.display()
						)
					})?;
					Ok(resolved)
				})
				.collect::<Result<Vec<_>>>()?
		} else {
			submissions.to_vec()
		};
		if paths.is_empty() {
			bail!("provide submissions, input.submissions, or input.students in assignment.toml");
		}
		Ok(Resolved {
			paths,
			roster: self.roster_table(assignment, roster_override)?,
			matching: matching.clone(),
		})
	}

	/// The table roster: `path` from the command line when given, otherwise
	/// `input.roster.path`, read with the `[input.roster]` layout either way.
	pub fn roster_table(
		&self,
		assignment: Option<&Path>,
		path: Option<&Path>,
	) -> Result<Option<Roster>> {
		if !self.students.is_empty() {
			bail!("input.students lists the class itself; it has no roster table");
		}
		let layout = self.roster.clone().unwrap_or_default();
		let path = match path {
			Some(path) => Some(path.to_path_buf()),
			None => layout
				.path
				.as_deref()
				.map(|path| relative_to(assignment, path))
				.transpose()?,
		};
		if self.roster.is_some() && path.is_none() {
			bail!("input.roster needs a path, or --roster");
		}
		path.map(|path| table::load(&path, &layout.options()))
			.transpose()
	}
}

/// A configured path, relative to the directory of the assignment file that names it.
fn relative_to(assignment: Option<&Path>, path: &Path) -> Result<PathBuf> {
	if path.as_os_str().is_empty() {
		bail!("an input path cannot be empty");
	}
	let base = assignment.and_then(Path::parent).unwrap_or(Path::new("."));
	std::path::absolute(base.join(path)).context("cannot resolve local input path")
}
