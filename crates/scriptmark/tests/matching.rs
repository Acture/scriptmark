//! P-673: preview, resolve an ambiguity with teacher configuration, then grade the
//! selected bytes. These exercise the real CLI and Python executor together.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::matching::{Config, OwnerOverride, State};
use scriptmark::models::{Cause, StudentReport};
use scriptmark::record::Record;

const SPEC: &str = r#"
[meta]
name = "double"
file = "solve.py"
function = "double"
language = "python"
[[cases]]
name = "basic"
args = [3]
expect = 6
"#;

struct Bench {
	dir: tempfile::TempDir,
}

impl Bench {
	fn new() -> Self {
		let bench = Self {
			dir: tempfile::tempdir().unwrap(),
		};
		bench.write("tests/test_double.toml", SPEC);
		bench
	}

	fn path(&self) -> &Path {
		self.dir.path()
	}

	fn write(&self, name: &str, content: &str) {
		let path: PathBuf = self.path().join(name);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, content).unwrap();
	}

	fn config(&self, rules: &str) {
		self.write(
			"assignment.toml",
			&format!("[assignment]\nname = 'hw'\n[grading]\nmissing_file = 'zero'\n{rules}"),
		);
	}

	fn run(&self, command: &str) -> Output {
		Command::new(env!("CARGO_BIN_EXE_scriptmark"))
			.current_dir(self.path())
			.args([command, "subs", "-t", "tests", "-o", "out/result.json"])
			.output()
			.unwrap()
	}

	fn successful(&self, command: &str) {
		let output = self.run(command);
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
	}

	fn preview(&self) -> serde_json::Value {
		self.successful("match");
		serde_json::from_slice(&std::fs::read(self.path().join("out/result.json")).unwrap())
			.unwrap()
	}

	fn grades(&self) -> Vec<StudentReport> {
		self.successful("grade");
		Record::load(&self.path().join("out/result.json"))
			.unwrap()
			.view(None)
			.unwrap()
			.reports
	}
}

const DIRECTORY: &str = "[[matching.students]]\nkind = 'directory'\nlevel = 0\n";

#[test]
fn nested_files_aliases_and_shared_file_are_traceable_without_mutating_sources() {
	let bench = Bench::new();
	bench.write(
		"tests/test_triple.toml",
		&SPEC
			.replace("double", "triple")
			.replace("expect = 6", "expect = 9"),
	);
	let source: &str = "raise RuntimeError('preview must not execute me')\ndef times_two(x):\n    return x * 2\ndef triple(x):\n    return x * 3\n";
	bench.write("subs/alice/deep/solve.py", source);
	bench.config(
		&(DIRECTORY.to_string()
			+ "[[matching.items]]\nid = 'double'\nfunctions = { double = ['times_two'] }\n"),
	);
	let first = bench.preview();
	assert_eq!(first["pending"], serde_json::json!([]));
	assert_eq!(
		first["items"][0]["functions"]["double"]["selected"],
		"times_two"
	);
	assert_eq!(first["items"][0]["owner"]["selected"], "alice");
	assert_eq!(
		first,
		bench.preview(),
		"same inputs/rules produce identical preview"
	);
	assert_eq!(
		std::fs::read_to_string(bench.path().join("subs/alice/deep/solve.py")).unwrap(),
		source
	);
	assert!(!bench.path().join("subs/alice/deep/__pycache__").exists());
	bench.write(
		"subs/alice/deep/solve.py",
		&source.replace("raise RuntimeError('preview must not execute me')\n", ""),
	);
	let grades = bench.grades();
	assert_eq!(grades[0].final_grade(), Some(100.0));
	assert_eq!(grades[0].matches.len(), 2, "one file can serve both items");
	assert_eq!(
		grades[0].matches[0].file.selected,
		grades[0].matches[1].file.selected
	);
	let input = grades[0].test_results[0].cases[0].input.as_ref().unwrap();
	assert_eq!(input.resolved.as_deref(), Some("times_two"));
	assert_eq!(input.matching.as_ref().unwrap().state, State::Matched);
}

#[test]
fn duplicate_filenames_withhold_grades_until_an_exact_override_selects_one() {
	let bench = Bench::new();
	bench.write("subs/alice/old/solve.py", "def double(x):\n    return 0\n");
	bench.write(
		"subs/alice/new/solve.py",
		"def double(x):\n    return x * 2\n",
	);
	bench.config(DIRECTORY);
	let preview = bench.preview();
	assert_eq!(preview["items"][0]["file"]["state"], "ambiguous");
	assert_eq!(
		preview["items"][0]["file"]["candidates"]
			.as_array()
			.unwrap()
			.len(),
		2
	);
	let grades = bench.grades();
	assert_eq!(
		grades[0].final_grade(),
		None,
		"missing_file=zero cannot score a conflict"
	);
	assert_eq!(
		grades[0].test_results[0].cases[0].cause,
		Some(Cause::Matching)
	);
	bench.config(&(DIRECTORY.to_string() + "[[matching.overrides]]\nstudent = 'local:alice'\nitem = 'double'\nfile = 'alice/new/solve.py'\n"));
	assert_eq!(bench.preview()["pending"], serde_json::json!([]));
	assert_eq!(bench.grades()[0].final_grade(), Some(100.0));
}

#[test]
fn even_one_fuzzy_function_is_reviewed_and_never_silently_called() {
	let bench = Bench::new();
	bench.write("subs/alice_solve.py", "def doubled(x):\n    return x * 2\n");
	assert_eq!(
		bench.preview()["items"][0]["functions"]["double"]["state"],
		"review"
	);
	let grades = bench.grades();
	assert_eq!(grades[0].final_grade(), None);
	assert_eq!(
		grades[0].test_results[0].cases[0].cause,
		Some(Cause::Matching)
	);
	let evidence = grades[0].test_results[0].cases[0]
		.input
		.as_ref()
		.unwrap()
		.matching
		.as_ref()
		.unwrap();
	assert_eq!(evidence.candidates[0].value, "doubled");
	bench.config("[[matching.overrides]]\nstudent = 'local:alice'\nitem = 'double'\nfunctions = { double = 'doubled' }\n");
	assert_eq!(bench.grades()[0].final_grade(), Some(100.0));
}

#[test]
fn aliases_are_candidates_and_explicit_function_override_wins_over_exact_name() {
	let bench = Bench::new();
	bench.write(
		"subs/alice_solve.py",
		"def times_two(x):\n    return x * 2\ndef multiply_two(x):\n    return x * 2\n",
	);
	let aliases = "[[matching.items]]\nid = 'double'\nfunctions = { double = ['times_two', 'multiply_two'] }\n";
	bench.config(aliases);
	assert_eq!(
		bench.preview()["items"][0]["functions"]["double"]["state"],
		"ambiguous"
	);
	assert_eq!(bench.grades()[0].final_grade(), None);
	bench.write(
		"subs/alice_solve.py",
		"def double(x):\n    return 0\ndef times_two(x):\n    return x * 2\n",
	);
	bench.config(&(aliases.to_string() + "[[matching.overrides]]\nstudent = 'local:alice'\nitem = 'double'\nfunctions = { double = 'times_two' }\n"));
	assert_eq!(bench.grades()[0].final_grade(), Some(100.0));
	bench.config(&(aliases.to_string() + "[[matching.overrides]]\nstudent = 'local:alice'\nitem = 'double'\nfunctions = { double = 'absent' }\n"));
	assert_eq!(
		bench.grades()[0].final_grade(),
		None,
		"invalid override must not fall back to double"
	);
}

#[test]
fn fuzzy_files_require_a_teacher_pattern_and_missing_files_stay_visible() {
	let bench = Bench::new();
	bench.write("subs/alice_final.py", "def double(x):\n    return x * 2\n");
	bench.write("subs/bob_other.py", "pass\n");
	let preview = bench.preview();
	assert_eq!(preview["items"][0]["file"]["state"], "review");
	assert_eq!(preview["items"][1]["file"]["state"], "missing");
	assert_eq!(bench.grades()[0].final_grade(), None);
	bench.config("[[matching.items]]\nid = 'double'\nfiles = ['*_final.py']\n");
	let grades = bench.grades();
	assert_eq!(grades[0].final_grade(), Some(100.0));
	assert_eq!(
		grades[1].final_grade(),
		Some(0.0),
		"declared missing_file policy still applies to absence"
	);
}

#[test]
fn student_rule_conflicts_are_previewable_and_explicit_ownership_resolves_them() {
	let bench = Bench::new();
	bench.write(
		"subs/alice/bob_solve.py",
		"def double(x):\n    return x * 2\n",
	);
	let conflict = DIRECTORY.to_string()
		+ "[[matching.students]]\nkind = 'pattern'\npattern = '{student}_*.py'\n";
	bench.config(&conflict);
	let preview = bench.preview();
	assert_eq!(preview["input"]["unmatched"].as_array().unwrap().len(), 1);
	bench.write("out/result.json", "prior results");
	assert!(!bench.run("grade").status.success());
	assert_eq!(
		std::fs::read_to_string(bench.path().join("out/result.json")).unwrap(),
		"prior results"
	);
	bench.config(
		&(conflict + "[[matching.owners]]\npath = 'alice/bob_solve.py'\nstudent = 'alice'\n"),
	);
	assert_eq!(bench.grades()[0].student_id, "local:alice");
	assert_eq!(bench.grades()[0].final_grade(), Some(100.0));
}

#[test]
fn filename_patterns_regex_and_canvas_download_prefixes_keep_identity_and_rules() {
	let bench = Bench::new();
	bench.write(
		"subs/00123_777_888_solve.py",
		"def double(x):\n    return x * 2\n",
	);
	let grades = bench.grades();
	assert_eq!(grades[0].student_id, "local:00123");
	assert_eq!(grades[0].final_grade(), Some(100.0));
	bench.config(
		"[[matching.students]]\nkind = 'regex'\nregex = '^(?P<student>[0-9]+)_[0-9]+_[0-9]+_.*$'\n",
	);
	assert_eq!(bench.grades()[0].student_id, "local:00123");
	bench.config("[[matching.students]]\nkind = 'pattern'\npattern = '{student}_777_888_*.py'\n");
	assert_eq!(bench.grades()[0].student_id, "local:00123");
}

#[test]
fn setup_function_matching_withholds_the_entire_scenario() {
	let bench = Bench::new();
	bench.write(
		"tests/test_double.toml",
		r#"
[meta]
name = 'double'
file = 'solve.py'
language = 'python'
[[scenarios]]
name = 'counter'
[[scenarios.setup]]
id = 'c'
function = 'Counter'
[[scenarios.steps]]
name = 'value'
attribute = 'value'
object = 'c'
expect = 6
"#,
	);
	bench.write("subs/alice_solve.py", "class Counters:\n    value = 6\n");
	let grades = bench.grades();
	assert_eq!(grades[0].final_grade(), None);
	assert_eq!(
		grades[0].matches[0].functions["Counter"].candidates[0].value,
		"Counters"
	);
	bench.config("[[matching.items]]\nid = 'double'\nfunctions = { Counter = ['Counters'] }\n");
	assert_eq!(bench.grades()[0].final_grade(), Some(100.0));
}

#[test]
fn an_archive_owner_override_applies_to_archive_contents_and_receipt() {
	use std::io::Write;
	let bench = Bench::new();
	std::fs::create_dir(bench.path().join("subs")).unwrap();
	let file = std::fs::File::create(bench.path().join("subs/mystery.zip")).unwrap();
	let mut zip = zip::ZipWriter::new(file);
	zip.start_file("deep/solve.py", zip::write::SimpleFileOptions::default())
		.unwrap();
	zip.write_all(b"def double(x):\n    return x * 2\n")
		.unwrap();
	zip.finish().unwrap();
	let config = Config {
		owners: vec![OwnerOverride {
			path: "mystery.zip".into(),
			student: "alice".into(),
		}],
		..Default::default()
	};
	let input = load_local_input(
		&[bench.path().join("subs")],
		LocalInputOptions {
			matching: Some(&config),
			..Default::default()
		},
	)
	.unwrap();
	assert_eq!(
		input.students.len(),
		1,
		"no phantom owner from the archive stem"
	);
	assert_eq!(input.students[0].key().to_string(), "local:alice");
	assert_eq!(input.students[0].files().len(), 1);
}

#[test]
fn invalid_rules_are_refused_before_execution() {
	let bench = Bench::new();
	bench.write(
		"subs/alice_solve.py",
		"raise RuntimeError('must not run')\n",
	);
	for invalid in [
		"[[matching.students]]\nkind = 'regex'\nregex = '(.*)'\n",
		"[[matching.students]]\nkind = 'pattern'\npattern = '*.py'\n",
		"[[matching.items]]\nid = 'double'\nfunctions = { typo = ['f'] }\n",
		"[[matching.items]]\nid = 'ghost'\nfiles = ['solve.py']\n",
	] {
		bench.config(invalid);
		assert!(!bench.run("grade").status.success());
		assert!(!bench.path().join("out/result.json").exists());
	}
}

#[test]
fn canvas_source_identity_and_attachment_origin_survive_teacher_file_rules() {
	use scriptmark::input::canvas::{CanvasPayload, DownloadedAttachment, normalize};
	use scriptmark::models::{Assignment, AttemptPolicy, FileOrigin};
	use std::collections::HashMap;
	let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hw1/canvas");
	let payload: CanvasPayload =
		serde_json::from_slice(&std::fs::read(fixture.join("assignment.json")).unwrap()).unwrap();
	let downloads = HashMap::from([(
		1001,
		Ok(DownloadedAttachment::file(
			fixture.join("files/1001/lab1.py"),
		)),
	)]);
	let input = normalize(
		&payload,
		None,
		&downloads,
		AttemptPolicy::Latest,
		Assignment::default(),
	);
	let specs = [scriptmark::spec_loader::load_spec_str(
		&SPEC.replace("solve.py", "lab1.py"),
		Path::new("."),
	)
	.unwrap()];
	let config: Config = toml::from_str("[[students]]\nkind = 'pattern'\npattern = '{student}*.py'\n[[owners]]\npath = 'lab1.py'\nstudent = 'wrong'\n").unwrap();
	let student = input
		.students
		.iter()
		.find(|s| s.identity.canvas_user_id == Some(101))
		.unwrap();
	let matched = scriptmark::matching::item_match(&config, student, &specs[0]);
	assert_eq!(matched.student, "2024010001");
	assert_eq!(
		matched.owner.as_ref().unwrap().selected.as_deref(),
		Some("2024010001")
	);
	assert!(matches!(
		matched.origin,
		Some(FileOrigin::Attachment {
			attachment_id: 1001,
			..
		})
	));
}
