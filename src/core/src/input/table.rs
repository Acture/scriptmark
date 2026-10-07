//! Roster tables share one typed reader and one identity validator.

use std::path::Path;

use anyhow::{Context, Result, bail};
use calamine::{Data, Reader, Xlsx};
use serde::{Deserialize, Serialize};

use crate::models::{DiagnosticKind, InputDiagnostic, SourceLocation, StudentKey, normalize_key};
use crate::roster::{Roster, RosterEntry, RosterSource};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
	pub sheet: Option<String>,
	/// One-based physical row containing the column headings.
	pub header_row: Option<usize>,
	pub columns: Option<Columns>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Columns {
	pub student_id: Column,
	pub name: Option<Column>,
	pub canvas_user_id: Option<Column>,
}

/// A header name, or a one-based column number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Column {
	Name(String),
	Number(usize),
}

impl Column {
	fn index(&self, header: &[Data]) -> Result<usize> {
		match self {
			Self::Number(n) if *n > 0 && *n <= header.len() => Ok(n - 1),
			Self::Number(n) => bail!("column {n} is outside the header; columns start at 1"),
			Self::Name(name) => {
				let matches: Vec<usize> = header
					.iter()
					.enumerate()
					.filter_map(|(i, cell)| {
						matches!(cell, Data::String(value) if value.trim() == name.trim())
							.then_some(i)
					})
					.collect();
				match matches.as_slice() {
					[i] if !name.trim().is_empty() => Ok(*i),
					[] => bail!("missing column '{name}'"),
					_ => bail!("column '{name}' is empty or appears more than once"),
				}
			}
		}
	}
}

struct Table {
	sheet: Option<String>,
	rows: Vec<(usize, Vec<Data>)>,
}

fn read(path: &Path, options: &Options) -> Result<Table> {
	match path
		.extension()
		.and_then(|s| s.to_str())
		.map(str::to_ascii_lowercase)
		.as_deref()
	{
		Some("csv") => {
			if options.sheet.is_some() {
				bail!("CSV has no worksheets; remove input.roster.sheet");
			}
			let mut reader = csv::ReaderBuilder::new()
				.has_headers(false)
				.flexible(true)
				.from_path(path)?;
			let mut rows = Vec::new();
			for record in reader.records() {
				let record = record?;
				let row = record.position().map_or(1, |p| p.line() as usize);
				rows.push((
					row,
					record.iter().map(|s| Data::String(s.to_owned())).collect(),
				));
			}
			Ok(Table { sheet: None, rows })
		}
		Some("xlsx") => {
			let mut book: Xlsx<_> = calamine::open_workbook(path)?;
			let sheet = match &options.sheet {
				Some(sheet) => sheet.clone(),
				None => match book.sheet_names().as_slice() {
					[sheet] => sheet.clone(),
					[] => bail!("workbook has no worksheets"),
					_ => bail!("workbook has multiple worksheets; set input.roster.sheet"),
				},
			};
			let range = book.worksheet_range(&sheet)?;
			let (row_offset, col_offset) = range.start().unwrap_or((0, 0));
			let rows = range
				.rows()
				.enumerate()
				.map(|(i, row)| {
					let mut cells = vec![Data::Empty; col_offset as usize];
					cells.extend_from_slice(row);
					(row_offset as usize + i + 1, cells)
				})
				.collect();
			Ok(Table {
				sheet: Some(sheet),
				rows,
			})
		}
		_ => bail!("roster must be a .csv or .xlsx file"),
	}
}

fn student_id(cell: Option<&Data>) -> Result<String> {
	match cell {
		None | Some(Data::Empty) => Ok(String::new()),
		Some(Data::String(value)) => Ok(normalize_key(value)),
		_ => bail!(
			"student_id must be a text cell; numeric cells cannot reliably preserve leading zeros"
		),
	}
}

fn canvas_id(cell: Option<&Data>) -> Result<Option<u64>> {
	let value = match cell {
		None | Some(Data::Empty) => return Ok(None),
		Some(Data::String(s)) if s.trim().is_empty() => return Ok(None),
		Some(Data::String(s)) => normalize_key(s)
			.parse::<u64>()
			.context("canvas_user_id must be a positive integer")?,
		Some(Data::Int(n)) if *n > 0 => u64::try_from(*n)?,
		Some(Data::Float(n))
			if n.is_finite() && *n > 0.0 && n.fract() == 0.0 && *n <= 9_007_199_254_740_991.0 =>
		{
			*n as u64
		}
		_ => bail!("canvas_user_id must be an exactly representable positive integer"),
	};
	if value == 0 {
		bail!("canvas_user_id must be positive");
	}
	Ok(Some(value))
}

pub(crate) fn valid_student_id(value: &str) -> Result<()> {
	if value.is_empty() {
		bail!("student_id is empty");
	}
	if let Some(prefix) = crate::models::RESERVED_KEY_PREFIXES
		.iter()
		.find(|p| value.starts_with(**p))
	{
		bail!("student id '{value}' starts with the reserved prefix '{prefix}'");
	}
	Ok(())
}

/// Read either format. Without a mapping, use name/id or name/unused/id/Canvas-id.
pub fn load(path: &Path, options: &Options) -> Result<Roster> {
	let table =
		read(path, options).with_context(|| format!("cannot read roster {}", path.display()))?;
	let header_row = options.header_row.unwrap_or(1);
	let header = table
		.rows
		.iter()
		.find(|(row, _)| *row == header_row)
		.map(|(_, cells)| cells)
		.with_context(|| {
			format!(
				"{} {:?}: header row {header_row} is missing (rows start at 1)",
				path.display(),
				table.sheet
			)
		})?;
	let mapping = options
		.columns
		.as_ref()
		.map(|columns| -> Result<(usize, Option<usize>, Option<usize>)> {
			let id = columns.student_id.index(header)?;
			let name = columns
				.name
				.as_ref()
				.map(|column| column.index(header))
				.transpose()?;
			let canvas = columns
				.canvas_user_id
				.as_ref()
				.map(|column| column.index(header))
				.transpose()?;
			let mut indexes = vec![id];
			indexes.extend(name);
			indexes.extend(canvas);
			indexes.sort_unstable();
			if indexes.windows(2).any(|pair| pair[0] == pair[1]) {
				bail!("each roster field must use a different column");
			}
			Ok((id, name, canvas))
		})
		.transpose()
		.with_context(|| format!("{} {:?}:{header_row}", path.display(), table.sheet))?;
	let mut entries = Vec::new();
	let mut diagnostics = Vec::new();
	for (row, cells) in &table.rows {
		if *row <= header_row {
			continue;
		}
		let location = SourceLocation {
			file: Some(path.to_path_buf()),
			sheet: table.sheet.clone(),
			row: Some(*row),
		};
		let parsed = (|| -> Result<RosterEntry> {
			if cells.len() != header.len() {
				bail!(
					"row has {} columns; header has {}",
					cells.len(),
					header.len()
				);
			}
			if header.len() < 2 && mapping.is_none() {
				bail!("need at least 2 columns, or an explicit column mapping");
			}
			let (id, name, canvas) = mapping.unwrap_or((
				if header.len() >= 3 { 2 } else { 1 },
				Some(0),
				(header.len() >= 4).then_some(3),
			));
			let number = student_id(cells.get(id))?;
			let uid = canvas_id(canvas.and_then(|i| cells.get(i)))?;
			let key = if number.is_empty() {
				StudentKey::CanvasUser(uid.context("row has no student id or Canvas user id")?)
			} else {
				valid_student_id(&number)?;
				StudentKey::Number(number)
			};
			let name = match name.and_then(|i| cells.get(i)) {
				None | Some(Data::Empty) => None,
				Some(Data::String(s)) => (!s.trim().is_empty()).then(|| s.trim().to_owned()),
				_ => bail!("name must be a text cell"),
			};
			Ok(RosterEntry {
				key,
				source: RosterSource::Supplied,
				name,
				canvas_user_id: uid,
				location: Some(location.clone()),
			})
		})();
		match parsed {
			Ok(entry) => entries.push(entry),
			Err(error) => diagnostics.push(
				InputDiagnostic::error(DiagnosticKind::UnusableRosterRow {
					reason: error.to_string(),
				})
				.at(location),
			),
		}
	}
	Ok(Roster::with_diagnostics(entries, diagnostics))
}
