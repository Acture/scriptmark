use super::{CheckInput, CheckOutput, Checker};

/// Exact equality checker (default).
pub struct ExactChecker;

impl Checker for ExactChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        if input.result == input.expected {
            CheckOutput {
                pass: true,
                message: String::new(),
            }
        } else {
            CheckOutput {
                pass: false,
                message: format!("expected {}, got {}", input.expected, input.result),
            }
        }
    }
}

/// Approximate floating-point checker.
pub struct ApproxChecker {
    pub tolerance: f64,
}

impl Checker for ApproxChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let actual = input.result.as_f64();
        let expected = input.expected.as_f64();

        match (actual, expected) {
            (Some(a), Some(e)) => {
                if (a - e).abs() <= self.tolerance {
                    CheckOutput {
                        pass: true,
                        message: String::new(),
                    }
                } else {
                    CheckOutput {
                        pass: false,
                        message: format!(
                            "expected {e} ± {}, got {a} (diff: {})",
                            self.tolerance,
                            (a - e).abs()
                        ),
                    }
                }
            }
            _ => CheckOutput {
                pass: false,
                message: format!(
                    "approx checker requires numeric values, got {} and {}",
                    input.result, input.expected
                ),
            },
        }
    }
}

/// Check that the result is a sorted array.
pub struct SortedChecker;

impl Checker for SortedChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let Some(arr) = input.result.as_array() else {
            return CheckOutput {
                pass: false,
                message: format!("expected array, got {}", input.result),
            };
        };

        let is_sorted = arr
            .windows(2)
            .all(|w| match (w[0].as_f64(), w[1].as_f64()) {
                (Some(a), Some(b)) => a <= b,
                _ => {
                    let a = w[0]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| w[0].to_string());
                    let b = w[1]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| w[1].to_string());
                    a <= b
                }
            });

        if is_sorted {
            CheckOutput {
                pass: true,
                message: String::new(),
            }
        } else {
            CheckOutput {
                pass: false,
                message: "array is not sorted".to_string(),
            }
        }
    }
}

/// Check that two arrays have the same elements (ignoring order).
pub struct SetEqChecker;

impl Checker for SetEqChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let (Some(actual), Some(expected)) = (input.result.as_array(), input.expected.as_array())
        else {
            return CheckOutput {
                pass: false,
                message: "set_eq checker requires arrays".to_string(),
            };
        };

        let mut actual_sorted: Vec<String> = actual.iter().map(|v| v.to_string()).collect();
        let mut expected_sorted: Vec<String> = expected.iter().map(|v| v.to_string()).collect();
        actual_sorted.sort();
        expected_sorted.sort();

        if actual_sorted == expected_sorted {
            CheckOutput {
                pass: true,
                message: String::new(),
            }
        } else {
            CheckOutput {
                pass: false,
                message: format!("sets differ: got {:?}, expected {:?}", actual, expected),
            }
        }
    }
}

/// Check that the result contains a substring.
pub struct ContainsChecker;

impl Checker for ContainsChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let actual_owned = input.result.to_string();
        let actual = input.result.as_str().unwrap_or(&actual_owned);
        let expected_owned = input.expected.to_string();
        let expected = input.expected.as_str().unwrap_or(&expected_owned);

        if actual.contains(expected) {
            CheckOutput {
                pass: true,
                message: String::new(),
            }
        } else {
            CheckOutput {
                pass: false,
                message: format!("output does not contain '{expected}'"),
            }
        }
    }
}

/// Check that the result matches a regex pattern.
pub struct RegexChecker {
    pub pattern: regex::Regex,
}

impl Checker for RegexChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let actual_owned = input.result.to_string();
        let actual = input.result.as_str().unwrap_or(&actual_owned);

        if self.pattern.is_match(actual) {
            CheckOutput {
                pass: true,
                message: String::new(),
            }
        } else {
            CheckOutput {
                pass: false,
                message: format!("output does not match pattern '{}'", self.pattern),
            }
        }
    }
}

/// Resolve a built-in checker by name.
pub fn resolve_builtin(name: &str, tolerance: Option<f64>) -> Option<Box<dyn Checker>> {
    match name {
        "exact" => Some(Box::new(ExactChecker)),
        "approx" => Some(Box::new(ApproxChecker {
            tolerance: tolerance.unwrap_or(1e-6),
        })),
        "sorted" => Some(Box::new(SortedChecker)),
        "set_eq" => Some(Box::new(SetEqChecker)),
        "contains" => Some(Box::new(ContainsChecker)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_exact_pass() {
        let checker = ExactChecker;
        let result = checker.check(&CheckInput {
            result: json!(5),
            expected: json!(5),
            context: json!({}),
        });
        assert!(result.pass);
    }

    #[test]
    fn test_exact_fail() {
        let checker = ExactChecker;
        let result = checker.check(&CheckInput {
            result: json!(3),
            expected: json!(5),
            context: json!({}),
        });
        assert!(!result.pass);
        assert!(result.message.contains("expected 5"));
    }

    #[test]
    fn test_approx_pass() {
        let checker = ApproxChecker { tolerance: 0.001 };
        let result = checker.check(&CheckInput {
            result: json!(1.23),
            expected: json!(1.238),
            context: json!({}),
        });
        assert!(!result.pass); // diff = 0.008 > 0.001

        let checker = ApproxChecker { tolerance: 0.01 };
        let result = checker.check(&CheckInput {
            result: json!(1.235),
            expected: json!(1.238),
            context: json!({}),
        });
        assert!(result.pass); // diff = 0.003 < 0.01
    }

    #[test]
    fn test_sorted_pass() {
        let checker = SortedChecker;
        let result = checker.check(&CheckInput {
            result: json!([1, 2, 3, 4]),
            expected: json!(null),
            context: json!({}),
        });
        assert!(result.pass);
    }

    #[test]
    fn test_sorted_fail() {
        let checker = SortedChecker;
        let result = checker.check(&CheckInput {
            result: json!([3, 1, 2]),
            expected: json!(null),
            context: json!({}),
        });
        assert!(!result.pass);
    }

    #[test]
    fn test_set_eq() {
        let checker = SetEqChecker;
        let result = checker.check(&CheckInput {
            result: json!([3, 1, 2]),
            expected: json!([1, 2, 3]),
            context: json!({}),
        });
        assert!(result.pass);
    }
}
