//! OSS-148: the grade sheet. `export` writes revision N of a grading record as CSV or as
//! XLSX from one table, so the two agree row for row and cell for cell: a 学号 stays text
//! with its leading zeros, scores are numbers, and a withheld grade is an empty cell where
//! a real zero is `0`. Run on `examples/bundles/grade_sheet`, as its comments say.

use std::path::{Path, PathBuf};
use std::process::Command;

use calamine::{Data, Reader, Xlsx};
use scriptmark::record::Record;

fn example() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/bundles/grade_sheet")
}

fn copy(from: &Path, to: &Path) {
	if from.is_dir() {
		std::fs::create_dir_all(to).unwrap();
		for entry in std::fs::read_dir(from).unwrap() {
			let entry = entry.unwrap();
			copy(&entry.path(), &to.join(entry.file_name()));
		}
	} else {
		std::fs::copy(from, to).unwrap();
	}
}

/// The example, graded with its roster.
fn graded() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	for name in [
		"assignment.toml",
		"regrade.toml",
		"roster.csv",
		"tests",
		"submissions",
	] {
		copy(&example().join(name), &dir.path().join(name));
	}
	scriptmark(
		dir.path(),
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"-r",
			"roster.csv",
			"-o",
			"out/results.json",
			"--archive",
			"out/archive",
		],
		None,
	);
	dir
}

fn scriptmark(dir: &Path, args: &[&str], path: Option<&str>) -> String {
	let mut command = Command::new(env!("CARGO_BIN_EXE_scriptmark"));
	command.current_dir(dir).args(args);
	if let Some(path) = path {
		command.env("PATH", path);
	}
	let output = command.output().unwrap();
	let text = format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(output.status.success(), "{args:?}: {text}");
	text
}

/// A CSV as spreadsheet software reads it: a byte order mark, then rows of fields.
fn read_csv(path: &Path) -> Vec<Vec<String>> {
	let text = std::fs::read_to_string(path).unwrap();
	let text = text
		.strip_prefix('\u{feff}')
		.expect("a UTF-8 byte order mark");
	csv::ReaderBuilder::new()
		.has_headers(false)
		.from_reader(text.as_bytes())
		.records()
		.map(|r| r.unwrap().iter().map(String::from).collect())
		.collect()
}

fn read_xlsx(path: &Path, sheet: &str) -> Vec<Vec<Data>> {
	let mut book: Xlsx<_> = calamine::open_workbook(path).unwrap();
	let range = book.worksheet_range(sheet).unwrap();
	assert_eq!(range.start(), Some((0, 0)), "{sheet} starts at A1");
	range.rows().map(<[Data]>::to_vec).collect()
}

/// The same table: every row and every cell — a number in a `numeric` column as the
/// number the CSV prints, anything else as the same text, an empty cell as an empty field.
/// A 学号 that looks like a number must still be text, and a score must be a number.
fn assert_same(csv: &[Vec<String>], xlsx: &[Vec<Data>], numeric: impl Fn(&str) -> bool) {
	assert_eq!(csv.len(), xlsx.len(), "rows");
	let header = &csv[0];
	for (r, (fields, cells)) in csv.iter().zip(xlsx).enumerate() {
		assert_eq!(fields.len(), cells.len(), "width of row {r}");
		for ((name, field), cell) in header.iter().zip(fields).zip(cells) {
			let at = format!("{name} in row {r}");
			let number = r > 0 && numeric(name);
			match cell {
				Data::Empty => assert_eq!(field, "", "{at}"),
				Data::String(s) if !number => assert_eq!(field, s, "{at}"),
				Data::Float(x) if number => assert_eq!(field, &format!("{x}"), "{at}"),
				Data::Int(i) if number => assert_eq!(field, &i.to_string(), "{at}"),
				other => panic!("{at} is {other:?}, from {field:?}"),
			}
		}
	}
}

/// The grades table's numbers: points, grades and the revision.
fn grade_number(column: &str) -> bool {
	matches!(
		column,
		"score" | "max" | "raw_grade" | "final_grade" | "lint" | "revision"
	) || column.ends_with("_score")
}

fn column(table: &[Vec<String>], name: &str) -> usize {
	table[0]
		.iter()
		.position(|h| h == name)
		.unwrap_or_else(|| panic!("no column {name} in {:?}", table[0]))
}

/// The row of `student`, by column name.
fn row<'a>(table: &'a [Vec<String>], student: &str) -> impl Fn(&str) -> &'a str {
	let at = table
		.iter()
		.position(|r| r[0] == student)
		.unwrap_or_else(|| panic!("no row for {student}"));
	let header = &table[0];
	let fields = &table[at];
	move |name: &str| {
		let col = header.iter().position(|h| h == name).unwrap();
		fields[col].as_str()
	}
}

/// What a sheet says once the one field that depends on where it was graded — the
/// evidence digest, which covers absolute paths and the interpreter — is set aside.
fn portable(table: &[Vec<String>]) -> Vec<Vec<String>> {
	let mut table = table.to_vec();
	let evidence = column(&table, "evidence");
	for row in table.iter_mut().skip(1) {
		row[evidence] = "<evidence>".into();
	}
	table
}

/// The checks every revision's sheet passes.
fn assert_a_grade_sheet(dir: &Path, revision: u32) -> Vec<Vec<String>> {
	let n = revision.to_string();
	let csv_path = dir.join(format!("out/grades-{n}.csv"));
	let xlsx_path = dir.join(format!("out/grades-{n}.xlsx"));
	for path in [&csv_path, &xlsx_path] {
		scriptmark(
			dir,
			&[
				"export",
				"out/results.json",
				"--revision",
				&n,
				"-o",
				path.to_str().unwrap(),
			],
			None,
		);
	}
	let csv = read_csv(&csv_path);
	assert_same(&csv, &read_xlsx(&xlsx_path, "grades"), grade_number);

	// One row per student, in the record's order, and the same bytes every time.
	let record = Record::load(&dir.join("out/results.json")).unwrap();
	let ids: Vec<&str> = csv[1..].iter().map(|r| r[0].as_str()).collect();
	let students: Vec<&str> = record
		.evidence
		.students
		.iter()
		.map(|s| s.student_id.as_str())
		.collect();
	assert_eq!(ids, students);
	assert!(ids.is_sorted(), "{ids:?}");
	let again = dir.join("out/again.csv");
	scriptmark(
		dir,
		&[
			"export",
			"out/results.json",
			"--revision",
			&n,
			"-o",
			"out/again.csv",
		],
		None,
	);
	assert_eq!(
		std::fs::read(&again).unwrap(),
		std::fs::read(&csv_path).unwrap()
	);

	// Which grading this is, on every row.
	for row in &csv[1..] {
		let field = |name| row[column(&csv, name)].as_str();
		assert_eq!(field("assignment"), "lab3 统计");
		assert_eq!(field("revision"), n);
		assert_eq!(field("evidence"), &record.digest[..12]);
	}

	// The items add up to the score, to the places a score is shown to.
	let items = ["mean_score", "parity_score"];
	for row in csv[1..]
		.iter()
		.filter(|r| r[column(&csv, "state")] == "graded")
	{
		let value = |name| row[column(&csv, name)].parse::<f64>().unwrap();
		let sum: f64 = items.iter().map(|i| value(i)).sum();
		let bound = (items.len() + 1) as f64 * 0.5e-4;
		assert!(
			(sum - value("score")).abs() <= bound + f64::EPSILON,
			"{row:?}"
		);
	}

	// The workbook's other sheets: what each item is, the cases behind every grade, and
	// where the grades came from.
	let items = read_xlsx(&xlsx_path, "items");
	assert_eq!(
		items[1..]
			.iter()
			.map(|r| (r[0].to_string(), r[1].to_string(), r[2].to_string()))
			.collect::<Vec<_>>(),
		[
			("mean".into(), "平均值".into(), "6".into()),
			("parity".into(), "奇偶判断".into(), "1".into()),
		]
	);
	let cases = read_xlsx(&xlsx_path, "cases");
	for id in &ids {
		assert!(
			cases[1..]
				.iter()
				.any(|r| r[0] == Data::String(id.to_string())),
			"no case rows for {id}"
		);
	}
	let about = read_xlsx(&xlsx_path, "record");
	let field = |name: &str| {
		about
			.iter()
			.find(|r| r[0] == Data::String(name.into()))
			.unwrap_or_else(|| panic!("no {name} in the record sheet"))[1]
			.to_string()
	};
	assert_eq!(field("evidence"), record.digest);
	assert_eq!(field("revision"), n);
	assert_eq!(
		field("revision_checksum"),
		record.revision(revision).unwrap().checksum
	);
	csv
}

#[test]
fn the_grade_sheet_is_one_table_as_csv_and_xlsx() {
	let temp = graded();
	let dir = temp.path();

	let csv = assert_a_grade_sheet(dir, 1);
	let zhang = row(&csv, "0012301");
	assert_eq!(
		(zhang("student_name"), zhang("state"), zhang("final_grade")),
		("张三", "graded", "100")
	);
	let li = row(&csv, "0012302");
	assert_eq!(
		(li("parity_score"), li("score"), li("final_grade")),
		("0.3333", "6.3333", "90.48")
	);
	// Wrong answers: a real 0, with no reason beside it.
	let wang = row(&csv, "0012303");
	assert_eq!(
		(
			wang("state"),
			wang("reason"),
			wang("score"),
			wang("final_grade")
		),
		("graded", "", "0", "0")
	);
	// Nothing handed in: no grade, and why.
	let zhao = row(&csv, "0012304");
	assert_eq!(
		(
			zhao("state"),
			zhao("reason"),
			zhao("score"),
			zhao("final_grade")
		),
		("withheld", "not_submitted", "", "")
	);
	// On no roster: no grade until someone reviews it.
	let unknown = row(&csv, "local:0012399");
	assert_eq!(
		(unknown("state"), unknown("reason"), unknown("final_grade")),
		("withheld", "pending_review", "")
	);
	assert_eq!(
		portable(&csv),
		portable(&read_csv(&example().join("expected/grades.csv")))
	);

	// `grade --archive` writes the same grades, and the cases the workbook holds.
	assert_eq!(
		std::fs::read(dir.join("out/archive/grades_tests.csv")).unwrap(),
		std::fs::read(dir.join("out/grades-1.csv")).unwrap()
	);
	let cases = read_csv(&dir.join("out/archive/archive_tests.csv"));
	assert_same(
		&cases,
		&read_xlsx(&dir.join("out/grades-1.xlsx"), "cases"),
		|column| column == "elapsed_ms",
	);
	// What a student printed reaches the sheet as it was, control characters and all.
	let qian = cases
		.iter()
		.find(|r| r[0] == "0012305" && r[column(&cases, "status")] == "error")
		.expect("0012305's mean raised");
	assert!(
		qian[column(&cases, "message")].contains("\u{1b}[31m没有实现"),
		"{qian:?}"
	);

	// Rescored with no interpreter on PATH: the missing submission is now a 0, and says why.
	scriptmark(
		dir,
		&[
			"rescore",
			"out/results.json",
			"--assignment",
			"regrade.toml",
		],
		Some(""),
	);
	let csv = assert_a_grade_sheet(dir, 2);
	let zhao = row(&csv, "0012304");
	assert_eq!(
		(
			zhao("state"),
			zhao("reason"),
			zhao("score"),
			zhao("final_grade")
		),
		("graded", "not_submitted", "0", "0")
	);
	assert_eq!(
		portable(&csv),
		portable(&read_csv(&example().join("expected/grades-rescored.csv")))
	);
}

#[test]
fn export_refuses_a_sheet_it_cannot_name() {
	let temp = graded();
	let dir = temp.path();
	let output = Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.args(["export", "out/results.json", "-o", "out/new/grades.ods"])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr).contains("is not a .csv or .xlsx file"),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(
		!dir.join("out/new").exists(),
		"refused before creating anything"
	);
}
