use std::path::Path;
use std::process::Command;

use crate::models::LintConfig;

/// Run a lint command on a student file and return its style score, 0 to 100.
///
/// Errs when the tool did not run properly — it could not start, or exited with a code
/// outside `ok_exit_codes`. A tool that failed has found nothing, and reading that as a
/// clean file would hand out full marks for a broken grader.
pub fn run_lint(config: &LintConfig, file_path: &Path) -> Result<f64, String> {
	let command = config
		.command
		.replace("{file}", &file_path.display().to_string());
	let parts: Vec<&str> = command.split_whitespace().collect();
	let Some((program, args)) = parts.split_first() else {
		return Err("the lint command is empty".into());
	};

	let out = Command::new(program)
		.args(args)
		.output()
		.map_err(|e| format!("could not run `{program}`: {e}"))?;
	if !out
		.status
		.code()
		.is_some_and(|code| config.ok_exit_codes.contains(&code))
	{
		return Err(format!(
			"`{command}` exited with {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr).trim()
		));
	}

	let stdout = String::from_utf8_lossy(&out.stdout);
	let warnings = match serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
		Ok(findings) => findings.len(),
		Err(_) => stdout.lines().filter(|l| !l.trim().is_empty()).count(),
	};
	Ok(style_score(warnings, config.max_warnings))
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
		assert_eq!(run_lint(&config("echo warning1", 10), &file), Ok(90.0));
		assert_eq!(run_lint(&config("true", 0), &file), Ok(100.0));
	}

	#[test]
	fn test_a_linter_reporting_findings_with_exit_1_is_scored() {
		let (_dir, file) = student_file();
		let sh = config("sh -c echo${IFS}w;exit${IFS}1", 10);
		assert_eq!(run_lint(&sh, &file), Ok(90.0));
	}

	#[test]
	fn test_a_linter_that_fails_is_an_error_not_100() {
		let (_dir, file) = student_file();
		let crashed = run_lint(&config("sh -c exit${IFS}2", 10), &file);
		assert!(crashed.unwrap_err().contains("exited with"));
		let missing = run_lint(&config("/nonexistent/ruff {file}", 10), &file);
		assert!(missing.unwrap_err().contains("could not run"));
		assert!(run_lint(&config("   ", 10), &file).is_err());
	}
}
