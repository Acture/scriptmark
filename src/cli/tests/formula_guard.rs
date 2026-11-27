//! OSS-290: student text in a CSV must not run as a spreadsheet formula.
//!
//! `grade --archive` writes one row per case, and `export -o grades.csv` writes the grade
//! sheet. Both prefix formula-like text with `'`. `export -o grades.xlsx` keeps the
//! original string, including a number that only looks like a formula.

use std::path::Path;
use std::process::Command;

use calamine::{Data, Reader, Xlsx};

struct Sample {
	name: &'static str,
	payload: &'static str,
	guarded: bool,
}

const SAMPLES: &[Sample] = &[
	Sample {
		name: "eq",
		payload: "=1+1",
		guarded: true,
	},
	Sample {
		name: "link",
		payload: "=HYPERLINK(\"http://example.com\",\"x\")",
		guarded: true,
	},
	Sample {
		name: "cmd",
		payload: "+CMD(...)",
		guarded: true,
	},
	Sample {
		name: "minus",
		payload: "-1+1",
		guarded: true,
	},
	Sample {
		name: "at",
		payload: "@SUM(1,2)",
		guarded: true,
	},
	Sample {
		name: "tab",
		payload: "\t=1+1",
		guarded: true,
	},
	Sample {
		name: "cr",
		payload: "\r=1+1",
		guarded: true,
	},
	Sample {
		name: "space",
		payload: " =1+1",
		guarded: true,
	},
	Sample {
		name: "lines",
		payload: "note\r\n=1+1",
		guarded: true,
	},
	Sample {
		name: "neg",
		payload: "-3",
		guarded: false,
	},
	Sample {
		name: "dec",
		payload: "-0.5",
		guarded: false,
	},
	Sample {
		name: "zero",
		payload: "0",
		guarded: false,
	},
	Sample {
		name: "fortytwo",
		payload: "42",
		guarded: false,
	},
	Sample {
		name: "plus",
		payload: "+10",
		guarded: false,
	},
	Sample {
		name: "text",
		payload: "hello",
		guarded: false,
	},
];

const RETURNED: &[(&str, &str)] = &[
	("ret_neg", "-3"),
	("ret_dec", "-0.5"),
	("ret_zero", "0"),
	("ret_fortytwo", "42"),
];

fn write(path: &Path, content: &str) {
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, content).unwrap();
}

fn toml_basic(text: &str) -> String {
	let mut out = String::from("\"");
	for c in text.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", u32::from(c))),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

fn spec() -> String {
	let mut text =
		String::from("[meta]\nname = \"sheet\"\nfile = \"lab.py\"\nlanguage = \"python\"\n\n");
	for sample in SAMPLES {
		text.push_str(&format!(
			"[[cases]]\nname = \"{}\"\nscript = true\nstdin = \"{}\"\nexpected_stdout = {}\n\n",
			sample.name,
			sample.name,
			toml_basic(sample.payload),
		));
	}
	for (name, function, expect) in [
		("returned", "give", "\"=1+1\""),
		("raised", "boom", "0"),
		("ret_neg", "num_neg", "-3"),
		("ret_dec", "num_dec", "-0.5"),
		("ret_zero", "num_zero", "0"),
		("ret_fortytwo", "num_fortytwo", "42"),
	] {
		text.push_str(&format!(
			"[[cases]]\nname = \"{name}\"\nfunction = \"{function}\"\nexpect = {expect}\n\n"
		));
	}
	text
}

fn student() -> String {
	let payloads: std::collections::BTreeMap<&str, &str> = SAMPLES
		.iter()
		.map(|sample| (sample.name, sample.payload))
		.collect();
	format!(
		"import json\nimport sys\n\nPAYLOADS = json.loads(r\"\"\"{}\"\"\")\n\n\
		 def boom():\n    raise ValueError(PAYLOADS[\"link\"])\n\n\
		 def give():\n    return PAYLOADS[\"eq\"]\n\n\
		 def num_neg():\n    return -3\n\n\
		 def num_dec():\n    return -0.5\n\n\
		 def num_zero():\n    return 0\n\n\
		 def num_fortytwo():\n    return 42\n\n\
		 if __name__ == \"__main__\":\n    \
		 sys.stdout.write(PAYLOADS[sys.stdin.read().strip()])\n",
		serde_json::to_string(&payloads).unwrap()
	)
}

fn scriptmark(dir: &Path, args: &[&str]) {
	let output = Command::new(env!("CARGO_BIN_EXE_scriptmark"))
		.current_dir(dir)
		.args(args)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{args:?}: {}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

fn read_csv(path: &Path) -> Vec<Vec<String>> {
	let text = std::fs::read_to_string(path).unwrap();
	let text = text
		.strip_prefix('\u{feff}')
		.expect("a UTF-8 byte order mark");
	csv::ReaderBuilder::new()
		.has_headers(false)
		.from_reader(text.as_bytes())
		.records()
		.map(|record| record.unwrap().iter().map(str::to_string).collect())
		.collect()
}

fn read_xlsx(path: &Path, sheet: &str) -> Vec<Vec<Data>> {
	let mut book: Xlsx<_> = calamine::open_workbook(path).unwrap();
	let range = book.worksheet_range(sheet).unwrap();
	range.rows().map(<[Data]>::to_vec).collect()
}

fn column(header: &[String], name: &str) -> usize {
	header
		.iter()
		.position(|cell| cell == name)
		.unwrap_or_else(|| panic!("no column {name}"))
}

fn csv_case<'a>(table: &'a [Vec<String>], name: &str) -> &'a [String] {
	let case_name = column(&table[0], "case_name");
	table
		.iter()
		.find(|row| row[case_name] == name)
		.unwrap_or_else(|| panic!("no case {name}"))
}

fn xlsx_column(header: &[Data], name: &str) -> usize {
	header
		.iter()
		.position(|cell| cell == &Data::String(name.into()))
		.unwrap_or_else(|| panic!("no column {name}"))
}

fn xlsx_case<'a>(table: &'a [Vec<Data>], name: &str) -> &'a [Data] {
	let case_name = xlsx_column(&table[0], "case_name");
	table
		.iter()
		.find(|row| row[case_name] == Data::String(name.into()))
		.unwrap_or_else(|| panic!("no case {name}"))
}

fn string_cell(cell: &Data) -> String {
	match cell {
		Data::String(text) => text.clone(),
		other => panic!("expected text, got {other:?}"),
	}
}

#[test]
fn csv_exports_neutralise_student_formulas_and_xlsx_keeps_them() {
	let dir = tempfile::tempdir().unwrap();
	let root = dir.path();
	write(
		&root.join("assignment.toml"),
		"[assignment]\nname = \"formula guard\"\n\n\
		 [[items]]\nid = \"sheet\"\ntitle = \"Sheet\"\npoints = 10\n\
		 aggregation = \"proportional\"\n",
	);
	write(
		&root.join("roster.csv"),
		"name,class,student_id\n=1+1,1,001\n",
	);
	write(&root.join("tests/test_sheet.toml"), &spec());
	write(&root.join("submissions/001_lab.py"), &student());

	scriptmark(
		root,
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
	);
	scriptmark(
		root,
		&["export", "out/results.json", "-o", "out/grades.csv"],
	);
	scriptmark(
		root,
		&["export", "out/results.json", "-o", "out/grades.xlsx"],
	);

	let archive = read_csv(&root.join("out/archive/archive_tests.csv"));
	let cases = read_xlsx(&root.join("out/grades.xlsx"), "cases");
	let actual = column(&archive[0], "actual");
	let expected = column(&archive[0], "expected");
	let message = column(&archive[0], "message");
	let name = column(&archive[0], "student_name");
	let x_actual = xlsx_column(&cases[0], "actual");
	let x_expected = xlsx_column(&cases[0], "expected");
	let x_message = xlsx_column(&cases[0], "message");
	let x_name = xlsx_column(&cases[0], "student_name");

	for sample in SAMPLES {
		let csv_row = csv_case(&archive, sample.name);
		let xlsx_row = xlsx_case(&cases, sample.name);
		let written = if sample.guarded {
			format!("'{}", sample.payload)
		} else {
			sample.payload.to_string()
		};
		assert_eq!(
			csv_row[column(&archive[0], "status")],
			"passed",
			"{}",
			sample.name
		);
		assert_eq!(csv_row[actual], written, "{}", sample.name);
		assert_eq!(csv_row[expected], written, "{}", sample.name);
		assert_eq!(
			string_cell(&xlsx_row[x_actual]),
			sample.payload,
			"{}",
			sample.name
		);
		assert_eq!(
			string_cell(&xlsx_row[x_expected]),
			sample.payload,
			"{}",
			sample.name
		);
		assert_eq!(csv_row[name], "'=1+1");
		assert_eq!(string_cell(&xlsx_row[x_name]), "=1+1");
	}

	let returned_csv = csv_case(&archive, "returned");
	let returned_xlsx = xlsx_case(&cases, "returned");
	let quoted = "\"=1+1\"";
	assert_eq!(returned_csv[actual], quoted);
	assert_eq!(returned_csv[expected], quoted);
	assert_eq!(string_cell(&returned_xlsx[x_actual]), quoted);
	assert_eq!(string_cell(&returned_xlsx[x_expected]), quoted);

	let raised = format!(
		"ValueError: {}",
		SAMPLES
			.iter()
			.find(|sample| sample.name == "link")
			.unwrap()
			.payload
	);
	let raised_csv = csv_case(&archive, "raised");
	let raised_xlsx = xlsx_case(&cases, "raised");
	assert_eq!(raised_csv[actual], raised);
	assert_eq!(raised_csv[message], raised);
	assert_eq!(raised_csv[expected], "0");
	assert_eq!(string_cell(&raised_xlsx[x_actual]), raised);
	assert_eq!(string_cell(&raised_xlsx[x_message]), raised);
	assert_eq!(string_cell(&raised_xlsx[x_expected]), "0");

	for (name, payload) in RETURNED {
		let csv_row = csv_case(&archive, name);
		let xlsx_row = xlsx_case(&cases, name);
		assert_eq!(csv_row[actual], *payload, "{name}");
		assert_eq!(csv_row[expected], *payload, "{name}");
		assert_eq!(string_cell(&xlsx_row[x_actual]), *payload, "{name}");
		assert_eq!(string_cell(&xlsx_row[x_expected]), *payload, "{name}");
	}

	let grades_path = root.join("out/grades.csv");
	assert_eq!(
		std::fs::read(&grades_path).unwrap(),
		std::fs::read(root.join("out/archive/grades_tests.csv")).unwrap()
	);
	let grades = read_csv(&grades_path);
	let grades_xlsx = read_xlsx(&root.join("out/grades.xlsx"), "grades");
	let student = grades
		.iter()
		.find(|row| row[0] == "001")
		.expect("grade row");
	let x_student = grades_xlsx
		.iter()
		.find(|row| row[0] == Data::String("001".into()))
		.expect("xlsx grade row");
	let grade_name = column(&grades[0], "student_name");
	let x_grade_name = xlsx_column(&grades_xlsx[0], "student_name");
	assert_eq!(student[grade_name], "'=1+1");
	assert_eq!(string_cell(&x_student[x_grade_name]), "=1+1");
	for column_name in ["score", "raw_grade", "final_grade"] {
		let value = &student[column(&grades[0], column_name)];
		assert!(
			!value.starts_with('\''),
			"{column_name} was rewritten: {value}"
		);
		value.parse::<f64>().unwrap();
	}
}
