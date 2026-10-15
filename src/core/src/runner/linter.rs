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
const LINT_TIMEOUT: Duration = Duration::from_secs(120);

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
) -> Result<f64, String> {
	// Split as a shell would, quotes included, before filling in the path: a path with a
	// space in it stays one argument.
	let parts = shlex::split(&config.command)
		.ok_or_else(|| format!("the lint command has an unclosed quote: {}", config.command))?;
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
	let copy = base.join("submission").join(
		file_path
			.file_name()
			.ok_or_else(|| format!("'{}' names no file", file_path.display()))?,
	);
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
	let args: Vec<String> = parts[1..]
		.iter()
		.map(|part| part.replace("{file}", &file))
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
	let mut path = std::env::join_paths(
		crate::runner::sandbox::system_programs()
			.iter()
			.map(|p| p.join("bin")),
	)
	.unwrap_or_default();
	if let Some(dir) = program.parent() {
		path = std::env::join_paths(
			std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path)),
		)
		.unwrap_or(path);
	}
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

	let out = crate::checker::python_checker::run_with_timeout(cmd, None, LINT_TIMEOUT)
		.map_err(|e| format!("could not run `{command}`: {e}"))?
		.ok_or_else(|| format!("`{command}` ran longer than {}s", LINT_TIMEOUT.as_secs()))?;
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

/// The program a command names: a path as given, or the first match on the grader's PATH.
fn locate(program: &str) -> Result<PathBuf, String> {
	let given = Path::new(program);
	if given.components().count() > 1 {
		return std::path::absolute(given).map_err(|e| format!("cannot find `{program}`: {e}"));
	}
	std::env::var_os("PATH")
		.iter()
		.flat_map(std::env::split_paths)
		.map(|dir| dir.join(program))
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

/// The directory a program is installed in: above its `bin` — a virtual environment or a
/// Homebrew keg — or its own. Never a prefix many programs share, such as `/`,
/// `/usr/local` or `/opt/homebrew`: those hold configuration and data in `etc` and `var`.
fn installation(real: &Path) -> PathBuf {
	let dir = real.parent().unwrap_or(real);
	let above = dir.parent().filter(|above| {
		matches!(
			dir.file_name().and_then(|n| n.to_str()),
			Some("bin" | "sbin")
		) && !["etc", "var"]
			.iter()
			.any(|shared| above.join(shared).exists())
	});
	above.unwrap_or(dir).to_path_buf()
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
		run_lint(config, file, file.parent().unwrap(), Isolation::Unisolated)
	}

	#[test]
	fn test_a_tool_is_read_from_its_own_installation_never_a_shared_prefix() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		for made in ["venv/bin", "prefix/bin", "prefix/etc"] {
			std::fs::create_dir_all(root.join(made)).unwrap();
		}
		assert_eq!(installation(&root.join("venv/bin/ruff")), root.join("venv"));
		assert_eq!(
			installation(&root.join("prefix/bin/ruff")),
			root.join("prefix/bin")
		);
		assert_eq!(installation(Path::new("/bin/sh")), PathBuf::from("/bin"));
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
