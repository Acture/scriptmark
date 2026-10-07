//! The same class through CSV, XLSX or a TOML file list, including the saved-record flow.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rust_xlsxwriter::Workbook;
use scriptmark_core::assignment;
use scriptmark_core::discovery::{LocalInputOptions, load_local_input};
use scriptmark_core::input::table::{self, Column, Columns, Options};
use scriptmark_core::models::{AssignmentInput, DiagnosticKind, SubmissionOutcome};
use scriptmark_core::record::Record;

fn write(root: &Path, name: &str, content: &str) -> PathBuf {
	let path = root.join(name);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(&path, content).unwrap();
	path
}

fn setup() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	write(
		dir.path(),
		"submissions/001_work.py",
		"def double(x):\n    return x * 2\n",
	);
	write(
		dir.path(),
		"submissions/002_work.py",
		"def double(x):\n    return x\n",
	);
	write(
		dir.path(),
		"tests/double.toml",
		"[meta]\nname = 'double'\nfile = 'work.py'\nfunction = 'double'\nlanguage = 'python'\n[[cases]]\nname = 'positive'\nargs = [3]\nexpect = 6\n",
	);
	write(
		dir.path(),
		"roster.csv",
		"class roster\n姓名,学号\n张三,001\n李四,002\n王五,003\n",
	);
	let mut book = Workbook::new();
	book.add_worksheet()
		.set_name("说明")
		.unwrap()
		.write_string(0, 0, "choose the roster worksheet")
		.unwrap();
	let sheet = book.add_worksheet();
	sheet.set_name("学生").unwrap();
	sheet.write_string(0, 0, "class roster").unwrap();
	for (i, (name, id)) in [
		("姓名", "学号"),
		("张三", "001"),
		("李四", "002"),
		("王五", "003"),
	]
	.into_iter()
	.enumerate()
	{
		sheet.write_string(i as u32 + 1, 0, name).unwrap();
		sheet.write_string(i as u32 + 1, 1, id).unwrap();
	}
	book.save(dir.path().join("roster.xlsx")).unwrap();
	for format in ["csv", "xlsx"] {
		let sheet = if format == "xlsx" {
			"sheet = '学生'\n"
		} else {
			""
		};
		write(
			dir.path(),
			&format!("{format}.toml"),
			&format!(
				"[assignment]\nname = 'local example'\n[input]\nsubmissions = ['submissions']\n[input.roster]\npath = 'roster.{format}'\n{sheet}header_row = 2\n[input.roster.columns]\nstudent_id = '学号'\nname = '姓名'\n"
			),
		);
	}
	write(
		dir.path(),
		"manifest.toml",
		"[assignment]\nname = 'local example'\n[[input.students]]\nstudent_id = '001'\nname = '张三'\nfiles = ['submissions/001_work.py']\n[[input.students]]\nstudent_id = '002'\nname = '李四'\nfiles = ['submissions/002_work.py']\n[[input.students]]\nstudent_id = '003'\nname = '王五'\nfiles = []\n",
	);
	dir
}

fn input(dir: &Path, config: &str) -> AssignmentInput {
	input_with(dir, config, &[])
}

fn input_with(dir: &Path, config: &str, submissions: &[PathBuf]) -> AssignmentInput {
	let declared = assignment::load(Some(&dir.join(config)), &dir.join("tests")).unwrap();
	let resolved = declared
		.input
		.resolve(
			declared.path.as_deref(),
			submissions,
			None,
			&declared.matching,
		)
		.unwrap();
	load_local_input(
		&resolved.paths,
		LocalInputOptions {
			assignment: declared.assignment,
			roster: resolved.roster.as_ref(),
			attempt_policy: declared.attempt_policy,
			matching: Some(&resolved.matching),
		},
	)
	.unwrap()
}

fn run(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.args(args)
		.output()
		.unwrap()
}

fn success(dir: &Path, args: &[&str]) {
	let output = run(dir, args);
	assert!(
		output.status.success(),
		"{args:?}\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

fn options() -> Options {
	Options {
		sheet: None,
		header_row: Some(2),
		columns: Some(Columns {
			student_id: Column::Name("学号".into()),
			name: Some(Column::Name("姓名".into())),
			canvas_user_id: None,
		}),
	}
}

#[test]
fn three_sources_preserve_identity_files_and_non_submitters() {
	let dir = setup();
	let inputs: Vec<AssignmentInput> = ["csv.toml", "xlsx.toml", "manifest.toml"]
		.iter()
		.map(|file| input(dir.path(), file))
		.collect();
	let signature = |input: &AssignmentInput| -> Vec<_> {
		input
			.students
			.iter()
			.map(|student| {
				(
					student.identity.clone(),
					student.outcome(),
					student
						.selected_attempt()
						.map(|attempt| {
							attempt
								.files
								.iter()
								.map(|file| file.path.clone())
								.collect::<Vec<_>>()
						})
						.unwrap_or_default(),
				)
			})
			.collect()
	};
	assert_eq!(signature(&inputs[0]), signature(&inputs[1]));
	assert_eq!(signature(&inputs[0]), signature(&inputs[2]));
	assert!(inputs.iter().all(|input| input.errors().next().is_none()));
	assert_eq!(
		inputs[0]
			.with_outcome(SubmissionOutcome::NotSubmitted)
			.count(),
		1
	);
	assert_eq!(inputs[0].students[0].identity.key.to_string(), "001");
}

#[test]
fn all_sources_grade_export_rescore_and_save_without_reloading_the_roster() {
	let dir = setup();
	let mut grades = Vec::new();
	for format in ["csv", "xlsx", "manifest"] {
		let config = format!("{format}.toml");
		let record = format!("out/{format}.json");
		success(
			dir.path(),
			&[
				"grade",
				"-t",
				"tests",
				"--assignment",
				&config,
				"-o",
				&record,
			],
		);
		success(
			dir.path(),
			&["export", &record, "-o", &format!("out/{format}.csv")],
		);
		let before = Record::load(&dir.path().join(&record)).unwrap();
		grades.push(
			before
				.view(None)
				.unwrap()
				.reports
				.iter()
				.map(|report| (report.student_id.clone(), report.final_grade()))
				.collect::<Vec<_>>(),
		);
		let content = std::fs::read_to_string(dir.path().join(&config)).unwrap();
		write(
			dir.path(),
			&config,
			&(content + "\n[grading]\nscale = 20\n"),
		);
		success(dir.path(), &["rescore", &record]);
		assert_eq!(
			Record::load(&dir.path().join(&record))
				.unwrap()
				.revisions
				.len(),
			2
		);
		std::fs::rename(
			dir.path().join(&config),
			dir.path().join(format!("{config}.saved")),
		)
		.unwrap();
		success(
			dir.path(),
			&["db", "save", &record, "--db", "out/grades.db"],
		);
	}
	assert_eq!(grades[0], grades[1]);
	assert_eq!(grades[0], grades[2]);
	let db = scriptmark::db::Database::open(&dir.path().join("out/grades.db")).unwrap();
	assert_eq!(
		db.get_student("001").unwrap().unwrap().name.as_deref(),
		Some("张三")
	);
}

#[test]
fn table_mapping_errors_and_numeric_ids_name_the_source() {
	let dir = setup();
	let mut configured = options();
	configured.columns.as_mut().unwrap().student_id = Column::Name("wrong heading".into());
	let error = format!(
		"{:#}",
		table::load(&dir.path().join("roster.csv"), &configured).unwrap_err()
	);
	assert!(
		error.contains("roster.csv:2: missing column 'wrong heading'"),
		"{error}"
	);
	let error = table::load(&dir.path().join("roster.xlsx"), &options()).unwrap_err();
	assert!(format!("{error:#}").contains("multiple worksheets"));
	let mut book = Workbook::new();
	let sheet = book.add_worksheet();
	sheet.set_name("学生").unwrap();
	sheet.write_string(0, 0, "title").unwrap();
	sheet.write_string(1, 0, "姓名").unwrap();
	sheet.write_string(1, 1, "学号").unwrap();
	sheet.write_string(2, 0, "张三").unwrap();
	sheet.write_number(2, 1, 123.0).unwrap();
	book.save(dir.path().join("numbers.xlsx")).unwrap();
	let roster = table::load(&dir.path().join("numbers.xlsx"), &options()).unwrap();
	let diagnostic = roster.errors().next().unwrap();
	assert_eq!(
		diagnostic.location.as_ref().unwrap().sheet.as_deref(),
		Some("学生")
	);
	assert_eq!(diagnostic.location.as_ref().unwrap().row, Some(3));
	assert!(diagnostic.to_string().contains("text cell"));
	assert!(diagnostic.to_string().contains("numbers.xlsx [学生]:3"));
}

#[test]
fn duplicate_missing_and_malformed_rows_remain_visible() {
	let dir = setup();
	write(
		dir.path(),
		"roster.csv",
		"title\n姓名,学号\n张三,001\n张三,001\n另一个人,001\n没有学号,\n缺列\n",
	);
	let roster = table::load(&dir.path().join("roster.csv"), &options()).unwrap();
	assert_eq!(roster.entries.len(), 1);
	let rows: Vec<usize> = roster
		.errors()
		.map(|diagnostic| diagnostic.location.as_ref().unwrap().row.unwrap())
		.collect();
	assert!(
		rows.contains(&5) && rows.contains(&6) && rows.contains(&7),
		"{rows:?}"
	);
	let output = run(
		dir.path(),
		&["grade", "-t", "tests", "--assignment", "csv.toml"],
	);
	assert!(!output.status.success());
	assert!(!dir.path().join("output/results.json").exists());
}

#[test]
fn manifest_paths_are_explicit_and_bad_paths_are_not_missing_submissions() {
	let dir = setup();
	write(
		dir.path(),
		"submissions/999_extra.py",
		"raise RuntimeError('must not be imported')\n",
	);
	assert_eq!(input(dir.path(), "manifest.toml").student_count(), 3);
	let original = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
	write(
		dir.path(),
		"manifest.toml",
		&original.replace("submissions/001_work.py", "absent.py"),
	);
	let imported = input(dir.path(), "manifest.toml");
	let diagnostic = imported.errors().next().unwrap();
	assert!(
		matches!(&diagnostic.kind, DiagnosticKind::InvalidSubmissionPath { student, .. } if student == "001")
	);
	assert!(diagnostic.location.as_ref().unwrap().row.is_some());
	let output = run(
		dir.path(),
		&["grade", "-t", "tests", "--assignment", "manifest.toml"],
	);
	assert!(!output.status.success());
	success(
		dir.path(),
		&["match", "-t", "tests", "--assignment", "manifest.toml"],
	);
}

#[test]
fn config_paths_survive_another_working_directory_and_changes_refuse_rescore() {
	let dir = setup();
	let elsewhere = tempfile::tempdir().unwrap();
	let tests = dir.path().join("tests");
	let config = dir.path().join("xlsx.toml");
	let output = dir.path().join("result.json");
	success(
		elsewhere.path(),
		&[
			"grade",
			"-t",
			tests.to_str().unwrap(),
			"--assignment",
			config.to_str().unwrap(),
			"-o",
			output.to_str().unwrap(),
		],
	);
	// With only the policy changed, the record rescores from anywhere: it names its
	// assignment absolutely.
	let original = std::fs::read_to_string(&config).unwrap() + "\n[grading]\nscale = 20\n";
	std::fs::write(&config, &original).unwrap();
	success(elsewhere.path(), &["rescore", output.to_str().unwrap()]);
	assert_eq!(Record::load(&output).unwrap().revisions.len(), 2);
	std::fs::write(
		&config,
		original
			.replace("roster.xlsx", "roster.csv")
			.replace("sheet = '学生'\n", ""),
	)
	.unwrap();
	write(
		dir.path(),
		"roster.csv",
		"title\n姓名,学号\n张三,099\n李四,002\n王五,003\n",
	);
	let result = run(elsewhere.path(), &["rescore", output.to_str().unwrap()]);
	assert!(!result.status.success());
	let stderr = String::from_utf8_lossy(&result.stderr);
	assert!(
		stderr.contains("student 001 is no longer in the input"),
		"{stderr}"
	);
	assert_eq!(Record::load(&output).unwrap().revisions.len(), 2);
}

#[test]
fn explicit_archive_ownership_and_conflicts_use_the_existing_matcher() {
	use std::io::Write;
	let dir = setup();
	let archive = dir.path().join("arbitrary.zip");
	let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
	zip.start_file("work.py", zip::write::SimpleFileOptions::default())
		.unwrap();
	zip.write_all(b"def double(x: int) -> int:\n    return x * 2\n")
		.unwrap();
	zip.finish().unwrap();
	let original = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
	write(
		dir.path(),
		"manifest.toml",
		&original.replace("submissions/001_work.py", "arbitrary.zip"),
	);
	let imported = input(dir.path(), "manifest.toml");
	assert!(imported.errors().next().is_none());
	assert_eq!(imported.students[0].identity.key.to_string(), "001");
	assert_eq!(
		imported.students[0].selected_attempt().unwrap().files.len(),
		1
	);
	success(
		dir.path(),
		&["run", "-t", "tests", "--assignment", "manifest.toml"],
	);
	let config = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
	write(
		dir.path(),
		"manifest.toml",
		&config.replace("submissions/002_work.py", "arbitrary.zip"),
	);
	let imported = input(dir.path(), "manifest.toml");
	assert!(
		imported
			.errors()
			.any(|diagnostic| diagnostic.to_string().contains("listed more than once"))
	);
	write(
		dir.path(),
		"manifest.toml",
		&(config + "\n[[matching.owners]]\npath = 'arbitrary.zip'\nstudent = '003'\n"),
	);
	assert!(
		input(dir.path(), "manifest.toml")
			.errors()
			.any(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::AmbiguousOwner { .. }))
	);
}

#[test]
fn mapped_columns_and_cli_overrides_do_not_silently_mix_input_modes() {
	let dir = setup();
	write(
		dir.path(),
		"other.csv",
		"名单\n编号,姓名\n001,张三\n002,李四\n003,王五\n",
	);
	let options = Options {
		sheet: None,
		header_row: Some(2),
		columns: Some(Columns {
			student_id: Column::Number(1),
			name: Some(Column::Name("姓名".into())),
			canvas_user_id: None,
		}),
	};
	assert_eq!(
		table::load(&dir.path().join("other.csv"), &options)
			.unwrap()
			.entries[0]
			.key
			.to_string(),
		"001"
	);
	let config = std::fs::read_to_string(dir.path().join("csv.toml"))
		.unwrap()
		.replace("'学号'", "'编号'");
	write(dir.path(), "csv.toml", &config);
	success(
		dir.path(),
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"--assignment",
			"csv.toml",
			"--roster",
			"other.csv",
		],
	);
	let output = run(
		dir.path(),
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"--assignment",
			"manifest.toml",
		],
	);
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("do not combine"));
}

#[test]
fn the_shipped_local_import_example_runs_as_documented() {
	let dir = tempfile::tempdir().unwrap();
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/bundles/local_import");
	for file in ["assignment.toml", "manifest.toml"] {
		success(
			dir.path(),
			&[
				"grade",
				"-t",
				root.join("tests").to_str().unwrap(),
				"--assignment",
				root.join(file).to_str().unwrap(),
				"-o",
				file,
			],
		);
		let record = Record::load(&dir.path().join(file)).unwrap();
		let reports = record.view(None).unwrap().reports;
		assert_eq!(
			reports
				.iter()
				.map(|report| report.final_grade())
				.collect::<Vec<_>>(),
			[Some(100.0), Some(0.0), None]
		);
	}
}

#[test]
fn rosters_read_outside_grading_use_the_same_mapping_and_refuse_errors() {
	let dir = setup();
	write(
		dir.path(),
		"plain.toml",
		"[assignment]\nname = 'local example'\n",
	);
	success(
		dir.path(),
		&[
			"grade",
			"submissions",
			"-t",
			"tests",
			"--assignment",
			"plain.toml",
			"-o",
			"out/plain.json",
		],
	);
	// The mapped table names the students, by --roster or by the assignment's own path.
	for args in [
		&["--roster", "roster.xlsx", "--assignment", "xlsx.toml"][..],
		&["--assignment", "csv.toml"][..],
	] {
		let output = run(
			dir.path(),
			&[&["summarize", "out/plain.json"][..], args].concat(),
		);
		assert!(output.status.success(), "{args:?}");
		assert!(String::from_utf8_lossy(&output.stdout).contains("张三"));
	}
	// Read with the default layout, the title row is taken for the header: refused, not a
	// summary that silently names nobody.
	let output = run(
		dir.path(),
		&["summarize", "out/plain.json", "--roster", "roster.csv"],
	);
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stdout).contains("roster.csv:3: roster row unusable"));

	let output = run(
		dir.path(),
		&["db", "import-roster", "roster.csv", "--db", "out/roster.db"],
	);
	assert!(!output.status.success());
	success(
		dir.path(),
		&[
			"db",
			"import-roster",
			"--assignment",
			"xlsx.toml",
			"--db",
			"out/roster.db",
		],
	);
	let name = |id: &str| {
		scriptmark::db::Database::open(&dir.path().join("out/roster.db"))
			.unwrap()
			.get_student(id)
			.unwrap()
			.unwrap()
			.name
	};
	assert_eq!(name("001").as_deref(), Some("张三"));

	// A record that knows the students but not their names keeps the imported names.
	let nameless = std::fs::read_to_string(dir.path().join("manifest.toml"))
		.unwrap()
		.replace("name = '张三'\n", "")
		.replace("name = '李四'\n", "")
		.replace("name = '王五'\n", "");
	write(dir.path(), "nameless.toml", &nameless);
	success(
		dir.path(),
		&[
			"grade",
			"-t",
			"tests",
			"--assignment",
			"nameless.toml",
			"-o",
			"out/nameless.json",
			"--db",
			"out/roster.db",
		],
	);
	assert_eq!(name("001").as_deref(), Some("张三"));
}

#[test]
fn explicit_files_are_never_noise_and_rules_see_only_their_names() {
	let dir = setup();
	write(
		dir.path(),
		"submissions/__main__.py",
		"def double(x):\n    return x * 2\n",
	);
	let manifest = std::fs::read_to_string(dir.path().join("manifest.toml"))
		.unwrap()
		.replace("submissions/001_work.py", "submissions/__main__.py");
	write(dir.path(), "manifest.toml", &manifest);
	let imported = input(dir.path(), "manifest.toml");
	assert!(imported.errors().next().is_none());
	assert_eq!(
		imported.students[0].selected_attempt().unwrap().files.len(),
		1
	);

	// A directory rule reads the directories below a scanned root. An explicit file has
	// none, so the filename default decides — never a directory above the file.
	write(
		dir.path(),
		"directory.toml",
		"[assignment]\nname = 'local example'\n[[matching.students]]\nkind = 'directory'\nlevel = 0\n",
	);
	let file = dir.path().join("submissions/002_work.py");
	let imported = input_with(dir.path(), "directory.toml", &[file]);
	assert!(imported.errors().next().is_none());
	assert_eq!(imported.students[0].identity.key.to_string(), "local:002");
}

#[test]
fn blank_rows_are_skipped_and_an_empty_table_is_refused() {
	let dir = setup();
	write(
		dir.path(),
		"roster.csv",
		"title\n姓名,学号\n张三,001\n,\n李四,002\n王五,003\n,\n,\n",
	);
	success(
		dir.path(),
		&["grade", "-t", "tests", "--assignment", "csv.toml"],
	);

	// A header row set below the data leaves nobody on the roster: refused, not a run in
	// which every student is merely "not on the roster".
	let roster = table::load(
		&dir.path().join("roster.csv"),
		&Options {
			header_row: Some(6),
			columns: None,
			..options()
		},
	)
	.unwrap();
	assert!(
		roster
			.errors()
			.any(|d| matches!(d.kind, DiagnosticKind::EmptyRoster { header_row: 6 }))
	);
	write(dir.path(), "roster.csv", "title\n姓名,学号\n,\n");
	let output = run(
		dir.path(),
		&["grade", "-t", "tests", "--assignment", "csv.toml"],
	);
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr)
			.contains("roster has no usable student rows below header row 2")
	);
}

#[test]
fn a_configured_submission_path_that_does_not_exist_names_the_assignment() {
	let dir = setup();
	let config = std::fs::read_to_string(dir.path().join("csv.toml"))
		.unwrap()
		.replace(
			"submissions = ['submissions']",
			"submissions = ['submisions']",
		);
	write(dir.path(), "csv.toml", &config);
	let output = run(
		dir.path(),
		&["grade", "-t", "tests", "--assignment", "csv.toml"],
	);
	assert!(!output.status.success());
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("csv.toml: cannot read input.submissions entry 'submisions'"),
		"{stderr}"
	);
}
