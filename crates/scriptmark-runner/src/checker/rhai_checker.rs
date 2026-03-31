use rhai::{Dynamic, Engine, Scope};

use super::{CheckInput, CheckOutput, Checker};

/// Checker that evaluates a Rhai inline expression.
///
/// The expression has access to `result`, `expected`, and `context` variables.
/// Must evaluate to a boolean: `true` = pass, `false` = fail.
pub struct RhaiChecker {
    pub expression: String,
}

impl RhaiChecker {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
        }
    }
}

/// Convert a serde_json::Value to a Rhai Dynamic value.
fn json_to_dynamic(value: &serde_json::Value) -> Dynamic {
    match value {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => Dynamic::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                Dynamic::from(f)
            } else {
                Dynamic::UNIT
            }
        }
        serde_json::Value::String(s) => Dynamic::from(s.clone()),
        serde_json::Value::Array(arr) => {
            let items: Vec<Dynamic> = arr.iter().map(json_to_dynamic).collect();
            Dynamic::from(items)
        }
        serde_json::Value::Object(obj) => {
            let mut map = rhai::Map::new();
            for (k, v) in obj {
                map.insert(k.clone().into(), json_to_dynamic(v));
            }
            Dynamic::from(map)
        }
    }
}

impl Checker for RhaiChecker {
    fn check(&self, input: &CheckInput) -> CheckOutput {
        let engine = Engine::new();
        let mut scope = Scope::new();

        scope.push_dynamic("result", json_to_dynamic(&input.result));
        scope.push_dynamic("expected", json_to_dynamic(&input.expected));
        scope.push_dynamic("context", json_to_dynamic(&input.context));

        match engine.eval_with_scope::<Dynamic>(&mut scope, &self.expression) {
            Ok(val) => {
                if let Ok(passed) = val.as_bool() {
                    if passed {
                        CheckOutput {
                            pass: true,
                            message: String::new(),
                        }
                    } else {
                        CheckOutput {
                            pass: false,
                            message: format!(
                                "Rhai check failed: `{}` evaluated to false",
                                self.expression
                            ),
                        }
                    }
                } else {
                    CheckOutput {
                        pass: false,
                        message: format!(
                            "Rhai expression must return bool, got: {}",
                            val.type_name()
                        ),
                    }
                }
            }
            Err(e) => CheckOutput {
                pass: false,
                message: format!("Rhai evaluation error: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_rhai_simple_true() {
        let checker = RhaiChecker::new("result > 0");
        let output = checker.check(&CheckInput {
            result: json!(42),
            expected: json!(null),
            context: json!({}),
        });
        assert!(output.pass);
    }

    #[test]
    fn test_rhai_simple_false() {
        let checker = RhaiChecker::new("result > 100");
        let output = checker.check(&CheckInput {
            result: json!(42),
            expected: json!(null),
            context: json!({}),
        });
        assert!(!output.pass);
        assert!(output.message.contains("evaluated to false"));
    }

    #[test]
    fn test_rhai_compare_with_expected() {
        let checker = RhaiChecker::new("result == expected");
        let output = checker.check(&CheckInput {
            result: json!(5),
            expected: json!(5),
            context: json!({}),
        });
        assert!(output.pass);
    }

    #[test]
    fn test_rhai_array_length() {
        let checker = RhaiChecker::new("result.len() > 2");
        let output = checker.check(&CheckInput {
            result: json!([1, 2, 3]),
            expected: json!(null),
            context: json!({}),
        });
        assert!(output.pass);
    }

    #[test]
    fn test_rhai_context_access() {
        let checker = RhaiChecker::new("result == context.answer");
        let output = checker.check(&CheckInput {
            result: json!(42),
            expected: json!(null),
            context: json!({"answer": 42}),
        });
        assert!(output.pass);
    }

    #[test]
    fn test_rhai_syntax_error() {
        let checker = RhaiChecker::new("invalid $$$ syntax");
        let output = checker.check(&CheckInput {
            result: json!(1),
            expected: json!(null),
            context: json!({}),
        });
        assert!(!output.pass);
        assert!(output.message.contains("Rhai evaluation error"));
    }

    #[test]
    fn test_rhai_non_bool_return() {
        let checker = RhaiChecker::new("result + 1");
        let output = checker.check(&CheckInput {
            result: json!(5),
            expected: json!(null),
            context: json!({}),
        });
        assert!(!output.pass);
        assert!(output.message.contains("must return bool"));
    }
}
