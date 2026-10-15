//! Style scores from a teacher's lint tool.
//!
//! The tool runs on a read-only copy of the student's file, in a private directory, with a
//! cleared environment and a deadline. Sandboxed, it runs in the teacher's sandbox: it reads
//! the system's programs, its own installation, the test bundle and that copy, writes only
//! its directory, and reaches no network.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::models::LintConfig;
use crate::runner::sandbox::{Confinement, Isolation};

/// How long a lint tool may run on one file.
pub const LINT_TIMEOUT: Duration = Duration::from_secs(120);

/// Run a lint command on a copy of `file_path` and return its style score, 0 to 100.
///
/// Errs when the tool did not run properly — it could not start, ran past its deadline, or
/// exited with a code outside `ok_exit_codes`. A tool that failed has found nothing, and
/// reading that as a clean file would hand out full marks for a broken grader.
pub fn run_lint(
	config: &LintConfig,
	file_path: &Path,
	bundle: &Path,
	isolation: Isolation,
	timeout: Duration,
) -> Result<f64, String> {
	// Split as a shell would, quotes included, before filling in the path: a path with a
	// space in it stays one argument.
	let parts = split(&config.command)?;
	let Some((program, _)) = parts.split_first() else {
		return Err("the lint command is empty".into());
	};
	let program = locate(program)?;

	let scratch = tempfile::Builder::new()
		.prefix("scriptmark-lint-")
		.tempdir()
		.and_then(|dir| std::fs::canonicalize(dir.path()).map(|path| (dir, path)))
		.map_err(|e| format!("could not stage the file to lint: {e}"))?;
	let (_keep, base) = scratch;
	let work = base.join("work");
	let name = file_path
		.file_name()
		.ok_or_else(|| format!("'{}' names no file", file_path.display()))?;
	let copy = base
		.join("submission")
		.join(safe_name(&name.to_string_lossy()));
	std::fs::create_dir_all(&work)
		.and_then(|_| std::fs::create_dir_all(copy.parent().expect("a parent")))
		.and_then(|_| std::fs::copy(file_path, &copy))
		.and_then(|_| {
			let mut readonly = std::fs::metadata(&copy)?.permissions();
			readonly.set_readonly(true);
			std::fs::set_permissions(&copy, readonly)
		})
		.map_err(|e| format!("could not stage the file to lint: {e}"))?;

	let file = copy.display().to_string();
	// `{bundle}` names the test bundle, for a configuration file kept in it.
	let bundle_dir = bundle.display().to_string();
	let args: Vec<String> = parts[1..]
		.iter()
		.map(|part| {
			part.replace("{file}", &file)
				.replace("{bundle}", &bundle_dir)
		})
		.collect();
	let command = std::iter::once(program.display().to_string())
		.chain(args.iter().cloned())
		.collect::<Vec<_>>()
		.join(" ");

	let mut cmd = if isolation.is_sandboxed() {
		let confinement = Confinement {
			read: crate::runner::sandbox::covering(
				crate::runner::sandbox::system_programs()
					.into_iter()
					.chain(tool_reads(&program))
					.chain([bundle.to_path_buf(), base.join("submission")])
					.filter_map(|p| std::fs::canonicalize(p).ok())
					.collect(),
			),
			write: vec![work.clone()],
			exec: Vec::new(),
			spawn: true,
			no_ffi: false,
		};
		crate::runner::sandbox::confined(&program, &confinement)
			.map_err(|e| format!("cannot sandbox `{command}`: {e}"))?
	} else {
		Command::new(&program)
	};
	// The tool's own directory, then the system's: what a wrapper script it runs may need.
	let path = std::env::join_paths(
		program.parent().map(Path::to_path_buf).into_iter().chain(
			PROGRAM_DIRS
				.iter()
				.map(PathBuf::from)
				.filter(|d| d.is_dir()),
		),
	)
	.unwrap_or_default();
	cmd.args(&args)
		.env_clear()
		.env("PATH", path)
		.env("HOME", &work)
		.env("TMPDIR", &work)
		.current_dir(&work)
		.stdin(Stdio::null());
	#[cfg(windows)]
	if let Some(root) = std::env::var_os("SYSTEMROOT") {
		cmd.env("SYSTEMROOT", root);
	}

	let out = crate::checker::python_checker::run_with_timeout(cmd, None, timeout)
		.map_err(|e| {
			format!(
				"could not run `{command}`: {}",
				crate::runner::sandbox::spawn_error(&e)
			)
		})?
		.ok_or_else(|| format!("`{command}` ran longer than {}s", timeout.as_secs()))?;
	if !out
		.status
		.code()
		.is_some_and(|code| config.ok_exit_codes.contains(&code))
	{
		return Err(format!(
			"`{command}` exited with {}: {}",
			out.status,
			out.stderr.trim()
		));
	}

	let warnings = match serde_json::from_str::<Vec<serde_json::Value>>(&out.stdout) {
		Ok(findings) => findings.len(),
		Err(_) => out.stdout.lines().filter(|l| !l.trim().is_empty()).count(),
	};
	Ok(style_score(warnings, config.max_warnings))
}

/// The student's file name with anything a shell would read as syntax replaced: the name
/// reaches the tool's arguments, and a teacher's command may hand it to `sh -c`. Letters,
/// digits, `.`, `_` and `-` stay, so a linter judging module names judges the same one.
fn safe_name(name: &str) -> String {
	let safe: String = name
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
				c
			} else {
				'_'
			}
		})
		.collect();
	// Never an option, and never `.` or `..`.
	if safe.starts_with(['-', '.']) {
		format!("_{safe}")
	} else {
		safe
	}
}

/// Where a tool finds the system's programs, in the order a shell would look.
#[cfg(unix)]
const PROGRAM_DIRS: &[&str] = &[
	"/usr/local/bin",
	"/opt/homebrew/bin",
	"/usr/bin",
	"/bin",
	"/usr/sbin",
	"/sbin",
];
#[cfg(not(unix))]
const PROGRAM_DIRS: &[&str] = &[];

/// A command split as a shell would, quotes included. On Windows, where a path is written
/// with backslashes a POSIX shell would read as escapes, on whitespace.
fn split(command: &str) -> Result<Vec<String>, String> {
	if cfg!(windows) {
		return Ok(command.split_whitespace().map(String::from).collect());
	}
	shlex::split(command)
		.ok_or_else(|| format!("the lint command has an unclosed quote: {command}"))
}

/// The program a command names: a path as given, or the first match on the grader's PATH
/// (with Windows' executable extensions there).
fn locate(program: &str) -> Result<PathBuf, String> {
	let given = Path::new(program);
	if given.components().count() > 1 {
		return std::path::absolute(given).map_err(|e| format!("cannot find `{program}`: {e}"));
	}
	let extensions: Vec<String> = if cfg!(windows) {
		std::iter::once(String::new())
			.chain(
				std::env::var("PATHEXT")
					.unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
					.split(';')
					.map(String::from),
			)
			.collect()
	} else {
		vec![String::new()]
	};
	std::env::var_os("PATH")
		.iter()
		.flat_map(std::env::split_paths)
		.flat_map(|dir| {
			extensions
				.iter()
				.map(move |ext| dir.join(format!("{program}{ext}")))
		})
		.find(|candidate| candidate.is_file())
		.ok_or_else(|| format!("could not run `{program}`: not found on PATH"))
}

/// What a tool reads to run: its installation — the directory above its `bin`, or its own —
/// and, for a script, its interpreter's, a Python interpreter's prefixes and packages
/// included.
fn tool_reads(program: &Path) -> Vec<PathBuf> {
	let Ok(real) = std::fs::canonicalize(program) else {
		return Vec::new();
	};
	let mut reads = vec![program.to_path_buf(), installation(&real)];
	let interpreter = std::fs::read(&real).ok().and_then(|bytes| {
		let line = bytes
			.split(|b| *b == b'\n')
			.next()?
			.strip_prefix(b"#!")?
			.to_vec();
		let line = String::from_utf8(line).ok()?;
		let mut words = line.split_whitespace();
		let first = words.next()?;
		// `#!/usr/bin/env python3` names its interpreter by the word after `env`.
		match Path::new(first).file_name()?.to_str()? {
			"env" => locate(words.next()?).ok(),
			_ => Some(PathBuf::from(first)),
		}
	});
	if let Some(interpreter) = interpreter {
		let is_python = interpreter
			.file_name()
			.and_then(|n| n.to_str())
			.is_some_and(|n| n.starts_with("python"));
		if is_python
			&& let Ok(confinement) = crate::runner::python::teacher_confinement(
				&interpreter.to_string_lossy(),
				Vec::new(),
				Vec::new(),
				true,
			) {
			reads.extend(confinement.read);
		}
		if let Ok(real) = std::fs::canonicalize(&interpreter) {
			reads.push(installation(&real));
		}
		reads.push(interpreter);
	}
	reads
}

/// What a program's installation is: the whole of it when it is self-contained — a virtual
/// environment, as `uv tool` and `pipx` make, or a Homebrew keg — and otherwise only the
/// program itself. A directory it merely sits in, such as `~/bin` or `/usr/local/bin`, may
/// hold anything.
fn installation(real: &Path) -> PathBuf {
	let dir = real.parent().unwrap_or(real);
	let above = dir.parent().filter(|above| {
		let in_bin = matches!(
			dir.file_name().and_then(|n| n.to_str()),
			Some("bin" | "sbin")
		);
		let venv = above.join("pyvenv.cfg").is_file();
		// …/Cellar/<formula>/<version>/bin/<program>
		let keg = above
			.parent()
			.and_then(Path::parent)
			.and_then(Path::file_name)
			.is_some_and(|n| n == "Cellar");
		in_bin && (venv || keg)
	});
	above.unwrap_or(real).to_path_buf()
}

/// `max(0, 1 - warnings / max_warnings) * 100`; with `max_warnings = 0`, any warning is 0.
fn style_score(warnings: usize, max_warnings: usize) -> f64 {
	if max_warnings == 0 {
		return if warnings == 0 { 100.0 } else { 0.0 };
	}
	((1.0 - warnings as f64 / max_warnings as f64) * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn config(command: &str, max_warnings: usize) -> LintConfig {
		LintConfig {
			command: command.into(),
			max_warnings,
			ok_exit_codes: vec![0, 1],
		}
	}

	fn student_file() -> (tempfile::TempDir, std::path::PathBuf) {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.py");
		std::fs::write(&file, "x = 1\n").unwrap();
		(dir, file)
	}

	/// Unconfined, so these run inside any sandbox the test itself is in; the sandboxed
	/// runs are exercised end to end in the integration tests.
	fn lint(config: &LintConfig, file: &Path) -> Result<f64, String> {
		run_lint(
			config,
			file,
			file.parent().unwrap(),
			Isolation::Unisolated,
			LINT_TIMEOUT,
		)
	}

	#[test]
	fn test_a_tool_is_read_whole_only_when_its_installation_is_its_own() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		for made in ["venv/bin", "Cellar/ruff/0.6/bin", "home/bin"] {
			std::fs::create_dir_all(root.join(made)).unwrap();
		}
		std::fs::write(root.join("venv/pyvenv.cfg"), "home = /usr/bin\n").unwrap();
		assert_eq!(installation(&root.join("venv/bin/ruff")), root.join("venv"));
		assert_eq!(
			installation(&root.join("Cellar/ruff/0.6/bin/ruff")),
			root.join("Cellar/ruff/0.6")
		);
		// `~/bin` holds whatever its owner put there: only the tool itself.
		assert_eq!(
			installation(&root.join("home/bin/ruff")),
			root.join("home/bin/ruff")
		);
		assert_eq!(installation(Path::new("/bin/sh")), PathBuf::from("/bin/sh"));
	}

	#[test]
	fn test_a_students_file_name_never_reaches_the_tool_as_syntax() {
		assert_eq!(safe_name("Lab5_1.py"), "Lab5_1.py");
		assert_eq!(safe_name("x'; rm -rf ~ #.py"), "x___rm_-rf____.py");
		assert_eq!(safe_name("-rf.py"), "_-rf.py");
		assert_eq!(safe_name("..py"), "_..py");
		assert_eq!(safe_name("实验一.py"), "___.py");
		// The tool sees the safe name, whatever the submission was called: one line.
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("a;echo injected;b.py");
		std::fs::write(&file, "x = 1\n").unwrap();
		let probe = config("sh -c 'echo {file}' sh", 10);
		assert_eq!(lint(&probe, &file), Ok(90.0));
	}

	#[test]
	fn test_a_tool_past_its_deadline_has_failed_and_its_children_go_with_it() {
		let (_dir, file) = student_file();
		let config = config("sh -c 'sleep 30 & sleep 30'", 10);
		let started = std::time::Instant::now();
		let late = run_lint(
			&config,
			&file,
			file.parent().unwrap(),
			Isolation::Unisolated,
			Duration::from_secs(1),
		);
		assert!(late.unwrap_err().contains("ran longer than 1s"));
		assert!(
			started.elapsed() < Duration::from_secs(10),
			"a child holding the pipes was ended with its group"
		);
	}

	#[test]
	fn test_bundle_names_the_test_bundle() {
		let (_dir, file) = student_file();
		let probe = config("sh -c 'test -d \"$1\" && echo found' sh {bundle}", 10);
		assert_eq!(lint(&probe, &file), Ok(90.0));
	}

	#[test]
	fn test_style_score() {
		assert!((style_score(3, 10) - 70.0).abs() < 1e-9);
		assert_eq!(style_score(5, 5), 0.0);
		assert_eq!(style_score(20, 5), 0.0, "clamped at 0");
		assert_eq!(style_score(0, 0), 100.0);
		assert_eq!(style_score(1, 0), 0.0);
	}

	#[test]
	fn test_run_lint_counts_output_lines() {
		let (_dir, file) = student_file();
		assert_eq!(lint(&config("echo warning1", 10), &file), Ok(90.0));
		assert_eq!(lint(&config("true", 0), &file), Ok(100.0));
	}

	#[test]
	fn test_a_path_with_a_space_stays_one_argument() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("Zhang San_1").join("lab one.py");
		std::fs::create_dir_all(file.parent().unwrap()).unwrap();
		std::fs::write(&file, "x = 1\n").unwrap();
		// `test -f` fails, exit 1 outside [0], unless it gets the path whole.
		let lint_config = LintConfig {
			ok_exit_codes: vec![0],
			..config("test -f {file}", 10)
		};
		assert_eq!(lint(&lint_config, &file), Ok(100.0));
	}

	#[test]
	fn test_quoted_arguments_are_split_as_a_shell_would() {
		let (_dir, file) = student_file();
		// Two arguments to `sh -c`, the script and $0: one warning line.
		let quoted = config("sh -c 'echo \"one warning\"' sh", 10);
		assert_eq!(lint(&quoted, &file), Ok(90.0));
		assert!(
			lint(&config("echo 'unclosed", 10), &file)
				.unwrap_err()
				.contains("unclosed quote")
		);
	}

	#[test]
	fn test_the_tool_sees_a_read_only_copy_and_no_environment() {
		let (_dir, file) = student_file();
		let probe = config(
			"sh -c 'test ! -w \"$1\" && test -z \"$CANVAS_TOKEN\" && echo seen' sh {file}",
			10,
		);
		assert_eq!(lint(&probe, &file), Ok(90.0));
	}

	#[test]
	fn test_a_linter_reporting_findings_with_exit_1_is_scored() {
		let (_dir, file) = student_file();
		let sh = config("sh -c 'echo w; exit 1'", 10);
		assert_eq!(lint(&sh, &file), Ok(90.0));
	}

	#[test]
	fn test_a_linter_that_fails_is_an_error_not_100() {
		let (_dir, file) = student_file();
		let crashed = lint(&config("sh -c 'exit 2'", 10), &file);
		assert!(crashed.unwrap_err().contains("exited with"));
		let missing = lint(&config("/nonexistent/ruff {file}", 10), &file);
		assert!(missing.unwrap_err().contains("could not run"));
		assert!(lint(&config("   ", 10), &file).is_err());
	}
}
