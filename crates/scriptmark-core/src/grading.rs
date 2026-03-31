use crate::models::{CurveConfig, CurveMethod, StudentReport, TestStatus};

/// Apply a grade curve to all student reports.
///
/// Modifies `final_grade` on each report based on the curve method.
pub fn apply_curve(reports: &mut [StudentReport], config: &CurveConfig) {
    let lower = config.lower_bound;
    let upper = config.upper_bound;

    for report in reports.iter_mut() {
        if report.status() == TestStatus::Missing {
            report.final_grade = Some(0.0);
            continue;
        }

        let rate = report.pass_rate();

        let curved = match config.method {
            CurveMethod::None => rate,
            CurveMethod::Linear => {
                // y = (x/100) * (max - min) + min
                (rate / 100.0) * (upper - lower) + lower
            }
            CurveMethod::Sqrt => {
                // y = sqrt(x) * multiplier + base
                let multiplier = (upper - lower) / 10.0;
                multiplier * rate.sqrt() + lower
            }
        };

        report.final_grade = Some(curved.clamp(lower, upper));
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
        }
    }

    #[test]
    fn test_curve_none() {
        let mut reports = vec![make_report(70)];
        apply_curve(
            &mut reports,
            &CurveConfig {
                method: CurveMethod::None,
                ..Default::default()
            },
        );
        assert!((reports[0].final_grade.unwrap() - 70.0).abs() < 0.1);
    }

    #[test]
    fn test_curve_linear() {
        let mut reports = vec![make_report(100)];
        let config = CurveConfig {
            method: CurveMethod::Linear,
            lower_bound: 60.0,
            upper_bound: 100.0,
        };
        apply_curve(&mut reports, &config);
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);

        let mut reports = vec![make_report(0)];
        apply_curve(&mut reports, &config);
        assert!((reports[0].final_grade.unwrap() - 60.0).abs() < 0.1);
    }

    #[test]
    fn test_curve_sqrt() {
        let mut reports = vec![make_report(100)];
        let config = CurveConfig {
            method: CurveMethod::Sqrt,
            lower_bound: 60.0,
            upper_bound: 100.0,
        };
        apply_curve(&mut reports, &config);
        // sqrt(100) = 10, multiplier = 4, so 4*10 + 60 = 100
        assert!((reports[0].final_grade.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_missing_student_gets_zero() {
        let mut reports = vec![StudentReport {
            student_id: "missing".to_string(),
            student_name: None,
            test_results: vec![],
            final_grade: None,
            backend_name: None,
        }];
        apply_curve(&mut reports, &CurveConfig::default());
        assert_eq!(reports[0].final_grade, Some(0.0));
    }
}
