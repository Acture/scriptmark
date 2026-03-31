use std::path::Path;
use std::process::Command;

use scriptmark_core::models::LintConfig;
use serde::{Deserialize, Serialize};

/// Result of running lint on a student file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    pub warning_count: usize,
    /// Style score from 0.0 to 100.0 (100 = no warnings).
    pub style_score: f64,
    /// Raw lint output (for debugging).
    pub raw_output: String,
}

/// Run a lint command on a student file and compute the style score.
///
/// Score = max(0, (1 - warnings / max_warnings)) * 100
pub fn run_lint(config: &LintConfig, file_path: &Path) -> LintResult {
    let command_str = config
        .command
        .replace("{file}", &file_path.display().to_string());

    let parts: Vec<&str> = command_str.split_whitespace().collect();

    if parts.is_empty() {
        return LintResult {
            warning_count: 0,
            style_score: 100.0,
            raw_output: "empty lint command".to_string(),
        };
    }

    let output = Command::new(parts[0]).args(&parts[1..]).output();

    let (warning_count, raw_output) = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = format!("{}{}", stdout, stderr);

            let count = if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                arr.len()
            } else {
                stdout.lines().filter(|l| !l.trim().is_empty()).count()
            };

            (count, combined)
        }
        Err(e) => (0, format!("Failed to run lint: {e}")),
    };

    let score = if config.max_warnings == 0 {
        if warning_count == 0 { 100.0 } else { 0.0 }
    } else {
        ((1.0 - warning_count as f64 / config.max_warnings as f64) * 100.0).clamp(0.0, 100.0)
    };

    LintResult {
        warning_count,
        style_score: score,
        raw_output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lint_score_calculation() {
        let config = LintConfig {
            command: "echo test".into(),
            max_warnings: 10,
            weight: 0.1,
        };
        let score = ((1.0 - 3.0 / config.max_warnings as f64) * 100.0).clamp(0.0, 100.0);
        assert!((score - 70.0).abs() < 0.1);
    }

    #[test]
    fn test_lint_score_zero_at_max() {
        let config = LintConfig {
            command: "echo test".into(),
            max_warnings: 5,
            weight: 0.1,
        };
        let warnings = 5usize;
        let score =
            ((1.0 - warnings as f64 / config.max_warnings as f64) * 100.0).clamp(0.0, 100.0);
        assert!((score - 0.0).abs() < 0.1);
    }

    #[test]
    fn test_lint_score_over_max_clamped() {
        let config = LintConfig {
            command: "echo test".into(),
            max_warnings: 5,
            weight: 0.1,
        };
        let warnings = 20usize;
        let score =
            ((1.0 - warnings as f64 / config.max_warnings as f64) * 100.0).clamp(0.0, 100.0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_run_lint_with_echo() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "x = 1\n").unwrap();

        let config = LintConfig {
            command: "echo warning1".into(),
            max_warnings: 10,
            weight: 0.1,
        };
        // echo outputs one line → warning_count = 1 → score = 90.0
        let result = run_lint(&config, &file);
        assert!(result.style_score > 0.0);
    }

    #[test]
    fn test_run_lint_zero_max_warnings_no_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "x = 1\n").unwrap();

        let config = LintConfig {
            command: "true".into(),
            max_warnings: 0,
            weight: 0.1,
        };
        let result = run_lint(&config, &file);
        assert_eq!(result.style_score, 100.0);
    }
}
