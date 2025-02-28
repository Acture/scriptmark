use runner;
use std::any::Any;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite::test_suite::{TestResult, TestSuiteTrait};

static SOLUTION_CODE: &str = include_str!("solutions/circle_area.py");

pub fn get_answer(input: f64) -> String {
    runner::python::run_code(SOLUTION_CODE, Some(&input.to_string()), None)
}
pub fn get_test_suite() -> Box<dyn TestSuiteTrait> {
    let test_inputs = util::generate(42, 20, 0.0, 100.0);
    let test_inputs_clone = test_inputs.clone();
    let answers: Vec<_> = test_inputs.iter().map(|input| get_answer(*input)).collect();
    let runner = move |path: &Path| {
        if !path.exists() {
            panic!("Test file not found: {}", path.display());
        }
        let content = fs::read_to_string(&path).expect("Failed to read file");
        let test_inputs_cloned = test_inputs.clone();
        test_inputs_cloned
            .into_iter()
            .map(|input| runner::python::run_code(content.clone(), Some(input.to_string()), None))
            .collect::<Vec<_>>()
    };
    let judge = |result: &[String], expected: &[String]| -> Vec<TestResult> {
        result
            .into_iter()
            .zip(expected.into_iter())
            .map(|(result, expected)| judge(result, expected))
            .collect::<Vec<TestResult>>()
    };
    Box::new(suite::test_suite::TestSuite::new(
        test_inputs_clone,
        answers,
        runner,
        judge,
    ))
}

pub fn judge(s: &str, t: &str) -> TestResult {
    let mut res = TestResult::builder().build();
    s.lines()
        .zip(t.lines())
        .enumerate()
        .for_each(|(i, (s_line, t_line))| match i {
            0 => {
                let s_extracted = util::extract_numbers::<f64>(s_line);
                let t_extracted = util::extract_numbers::<f64>(t_line);

                if s_extracted.len() != 1 || t_extracted.len() != 1 {
                    res.infos
                        .get_or_insert_with(HashMap::new)
                        .entry("More Than One Number".to_string())
                        .or_insert(format!(
                            "Expected: 1, Got: s: {}, t: {}",
                            s_extracted.len(),
                            t_extracted.len()
                        ));
                }
                let s_area = match s_extracted.first() {
                    Some(value) => *value,
                    None => {
                        res.passed = false;
                        res.infos
                            .get_or_insert_with(HashMap::new)
                            .entry("Area".to_string())
                            .or_insert(format!(
                                "Failed to extract number from line {}: {:?}",
                                i, s
                            ));
                        return;
                    }
                };
                let t_area = match t_extracted.first() {
                    Some(value) => *value,
                    None => {
                        res.passed = false;
                        res.infos
                            .get_or_insert_with(HashMap::new)
                            .entry("Area".to_string())
                            .or_insert(format!(
                                "Failed to extract number from line {}: {:?}",
                                i, t
                            ));
                        return;
                    }
                };
                let offset_percent = 0.0001;
                if (s_area - t_area).abs() > offset_percent * s_area {
                    res.passed = false;
                    res.infos
                        .get_or_insert_with(HashMap::new)
                        .entry("Area".to_string())
                        .or_insert(format!("Expected: <{}>, Got: <{}>", t_area, s_area));
                } else {
                    res.passed = true;
                }
            }
            1 => {
                let s_count = match util::extract_numbers::<f64>(t_line).pop() {
                    Some(value) => value,
                    None => {
                        res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
                        res.additional_infos
                            .get_or_insert_with(HashMap::new)
                            .entry("Count".to_string())
                            .or_insert(format!(
                                "Failed to extract number from line {}: {:?}",
                                i, s
                            ));
                        return;
                    }
                };
                let t_count = match util::extract_numbers::<f64>(t_line).pop() {
                    Some(value) => value,
                    None => {
                        res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
                        res.additional_infos
                            .get_or_insert_with(HashMap::new)
                            .entry("Count".to_string())
                            .or_insert(format!(
                                "Failed to extract number from line {}: {:?}",
                                i, t
                            ));
                        return;
                    }
                };
                if s_count != t_count {
                    res.additional_status = Some(suite::test_suite::AdditionalStatus::Partial);
                    res.additional_infos
                        .get_or_insert_with(HashMap::new)
                        .entry("Count".to_string())
                        .or_insert(format!("Expected: <{}>, Got: <{}>", t_count, s_count));
                } else {
                    res.additional_status = Some(suite::test_suite::AdditionalStatus::Full);
                }
            }
            _ => {
                res.passed = false;
                res.infos
                    .get_or_insert_with(HashMap::new)
                    .entry("Extra Lines".to_string())
                    .or_insert(format!("Extra line: {}", s_line));
            }
        });
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_answer() {
        let input = 3.14;
        let answer = get_answer(input);
        assert_eq!(
            answer,
            "Radius？ Area is:  30.974846927333928\nIts integral part is a 2-digit number.\n"
        );
    }
}
