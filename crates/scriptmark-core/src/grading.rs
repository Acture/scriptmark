use rhai::{Dynamic, Engine, Scope};

use crate::models::{FormulaPolicy, GradingPolicy, StudentReport, TemplatePolicy, TestStatus};

/// Apply a grading policy to all student reports.
///
/// Modifies `final_grade` on each report based on the policy.
pub fn apply_grading(reports: &mut [StudentReport], policy: &GradingPolicy) {
    match policy {
        GradingPolicy::Template(t) => apply_template(reports, t),
        GradingPolicy::Formula(f) => apply_formula(reports, f),
    }
}

fn apply_template(reports: &mut [StudentReport], config: &TemplatePolicy) {
    let lower = config.lower;
    let upper = config.upper;

    for report in reports.iter_mut() {
        if report.status() == TestStatus::Missing {
            report.final_grade = Some(0.0);
            continue;
        }

        let rate = report.pass_rate();

        let grade = match config.template.as_str() {
            "none" => rate,
            "linear" => lower + (rate / 100.0) * (upper - lower),
            "sqrt" => {
                let multiplier = (upper - lower) / 10.0;
                lower + multiplier * rate.sqrt()
            }
            "log" => lower + (1.0 + rate).ln() / (101.0_f64).ln() * (upper - lower),
            "strict" => {
                if rate >= 100.0 {
                    upper
                } else if rate >= 80.0 {
                    lower + (rate - 80.0) / 20.0 * (upper - lower)
                } else {
                    lower
                }
            }
            _ => rate, // unknown template, fall back to raw rate
        };

        report.final_grade = Some(grade.clamp(0.0, upper));

        // Blend lint score if available
        if let Some(lint) = report.lint_score {
            let test_weight = 0.9;
            let blended = report.final_grade.unwrap() * test_weight
                + lint * (1.0 - test_weight) / 100.0 * upper;
            report.final_grade = Some(blended.clamp(0.0, upper));
        }
    }
}

fn apply_formula(reports: &mut [StudentReport], config: &FormulaPolicy) {
    let engine = Engine::new();

    for report in reports.iter_mut() {
        if report.status() == TestStatus::Missing {
            report.final_grade = Some(0.0);
            continue;
        }

        let mut scope = Scope::new();
        scope.push("rate", report.pass_rate());
        scope.push("passed", report.total_passed() as i64);
        scope.push("total", report.total_cases() as i64);
        scope.push("lint_score", report.lint_score.unwrap_or(0.0));

        match engine.eval_with_scope::<Dynamic>(&mut scope, &config.formula) {
            Ok(val) => {
                let grade = if let Ok(f) = val.as_float() {
                    f
                } else if let Ok(i) = val.as_int() {
                    i as f64
                } else {
                    0.0
                };
                report.final_grade = Some(grade.clamp(0.0, 100.0));
            }
            Err(e) => {
                eprintln!("Grading formula error for {}: {}", report.student_id, e);
                report.final_grade = Some(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CaseResult, TestResult};

    fn make_report(pass_rate_pct: usize) -> StudentReport {
        let total = 10;
        let passed = pass_rate_pct * total / 100;
        let cases: Vec<CaseResult> = (0..total)
            .map(|i| CaseResult {
                case_name: format!("case_{i}"),
                status: if i < passed {
                    TestStatus::Passed
                } else {
                    TestStatus::Failed
                },
                actual: None,
                expected: None,
                failure: None,
                elapsed_ms: None,
            })
            .collect();

        StudentReport {
            student_id: "test".to_string(),
            student_name: None,
            test_results: vec![TestResult {
                spec_name: "test".to_string(),
                cases,
            }],
            final_grade: None,
            backend_name: None,
            lint_score: None,
        }
    }

    fn template(name: &str) -> GradingPolicy {
        GradingPolicy::Template(TemplatePolicy {
            template: name.to_string(),
            lower: 60.0,
            upper: 100.0,
        })
    }

    #[test]
    fn test_template_none() {
        let mut reports = vec![make_report(70)];
        apply_grading(&mut reports, &template("none"));
        assert!((reports[0].final_grade.unwrap() - 70.0).abs() < 0.1);
    }

    #[test]
    fn test_template_linear() {
        let policy = template("linear");

        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);

        let mut reports = vec![make_report(0)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 60.0).abs() < 0.1);
    }

    #[test]
    fn test_template_sqrt() {
        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &template("sqrt"));
        // sqrt(100) = 10, multiplier = 4, so 4*10 + 60 = 100
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_template_log() {
        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &template("log"));
        // ln(101)/ln(101) * 40 + 60 = 100
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_template_strict() {
        let policy = template("strict");

        // 100% pass rate -> upper
        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);

        // 50% pass rate -> lower (below 80%)
        let mut reports = vec![make_report(50)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 60.0).abs() < 0.1);
    }

    #[test]
    fn test_missing_student_gets_zero() {
        let mut reports = vec![StudentReport {
            student_id: "missing".to_string(),
            student_name: None,
            test_results: vec![],
            final_grade: None,
            backend_name: None,
            lint_score: None,
        }];
        apply_grading(&mut reports, &GradingPolicy::default());
        assert_eq!(reports[0].final_grade, Some(0.0));
    }

    #[test]
    fn test_formula_basic() {
        let policy = GradingPolicy::Formula(FormulaPolicy {
            formula: "rate * 0.9 + 10.0".to_string(),
        });
        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &policy);
        // 100 * 0.9 + 10 = 100
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_formula_uses_variables() {
        let policy = GradingPolicy::Formula(FormulaPolicy {
            formula: "if passed == total { 100.0 } else { 50.0 }".to_string(),
        });

        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);

        let mut reports = vec![make_report(50)];
        apply_grading(&mut reports, &policy);
        assert!((reports[0].final_grade.unwrap() - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_formula_error_gives_zero() {
        let policy = GradingPolicy::Formula(FormulaPolicy {
            formula: "undefined_var + 1".to_string(),
        });
        let mut reports = vec![make_report(100)];
        apply_grading(&mut reports, &policy);
        assert_eq!(reports[0].final_grade, Some(0.0));
    }
}
