use rhai::{Dynamic, Engine, Scope};

use super::{CheckError, CheckInput, CheckOutput, Checker};

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

/// The names a Rhai check sees.
pub const VARIABLES: [&str; 3] = ["result", "expected", "context"];

/// Compile a Rhai expression with strict variables, so an undefined name is refused now
/// rather than failing on every student later.
pub fn compile(expr: &str, names: &[&str]) -> Result<(), String> {
	let mut engine = engine();
	engine.set_strict_variables(true);
	let mut scope = Scope::new();
	for name in names {
		scope.push_dynamic(*name, Dynamic::UNIT);
	}
	engine
		.compile_with_scope(&scope, expr)
		.map(|_| ())
		.map_err(|e| e.to_string())
}

/// Whether a check's expression refers to `name`: with every other variable in scope it
/// no longer compiles.
pub fn refers_to(expr: &str, name: &str) -> bool {
	let others: Vec<&str> = VARIABLES.into_iter().filter(|v| *v != name).collect();
	compile(expr, &VARIABLES).is_ok() && compile(expr, &others).is_err()
}

/// The one Rhai engine configuration: bounded, so a teacher expression that loops cannot
/// hang a run — it fails, and a check that cannot finish is the teacher's to fix.
pub fn engine() -> Engine {
	let mut engine = Engine::new();
	engine.set_max_operations(1_000_000);
	engine.set_max_call_levels(64);
	engine.set_max_expr_depths(64, 32);
	engine
}

/// Convert a serde_json::Value to a Rhai Dynamic value.
pub fn json_to_dynamic(value: &serde_json::Value) -> Dynamic {
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
	fn check(&self, input: &CheckInput) -> Result<CheckOutput, CheckError> {
		let engine = engine();
		let mut scope = Scope::new();

		scope.push_dynamic("result", json_to_dynamic(&input.result));
		scope.push_dynamic("expected", json_to_dynamic(&input.expected));
		scope.push_dynamic("context", json_to_dynamic(&input.context));

		// An expression that cannot evaluate, or does not say yes or no, has not decided —
		// even when a malformed answer is what tripped it. That is the teacher's to resolve.
		let value = engine
			.eval_with_scope::<Dynamic>(&mut scope, &self.expression)
			.map_err(|e| {
				CheckError::teacher(format!(
					"rhai check `{}` could not evaluate: {e}",
					self.expression
				))
			})?;
		let passed = value.as_bool().map_err(|_| {
			CheckError::teacher(format!(
				"rhai check `{}` returned {}, not a bool",
				self.expression,
				value.type_name()
			))
		})?;
		Ok(CheckOutput {
			pass: passed,
			message: if passed {
				String::new()
			} else {
				format!("`{}` is false", self.expression)
			},
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn test_rhai_simple_true() {
		let checker = RhaiChecker::new("result > 0");
		let output = checker
			.check(&CheckInput {
				result: json!(42),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap();
		assert!(output.pass);
	}

	#[test]
	fn test_rhai_simple_false() {
		let checker = RhaiChecker::new("result > 100");
		let output = checker
			.check(&CheckInput {
				result: json!(42),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap();
		assert!(!output.pass);
		assert!(output.message.contains("is false"));
	}

	#[test]
	fn test_refers_to_sees_a_variable_only_where_it_is_used() {
		assert!(refers_to("result == expected", "expected"));
		assert!(refers_to("let e = expected; result == e", "expected"));
		assert!(!refers_to("result.len() > 2", "expected"));
		assert!(
			!refers_to("\"expected\" == result", "expected"),
			"a string is not the variable"
		);
		assert!(
			!refers_to("result ==", "expected"),
			"what does not compile refers to nothing"
		);
	}

	#[test]
	fn test_rhai_compare_with_expected() {
		let checker = RhaiChecker::new("result == expected");
		let output = checker
			.check(&CheckInput {
				result: json!(5),
				expected: json!(5),
				context: json!({}),
			})
			.unwrap();
		assert!(output.pass);
	}

	#[test]
	fn test_rhai_array_length() {
		let checker = RhaiChecker::new("result.len() > 2");
		let output = checker
			.check(&CheckInput {
				result: json!([1, 2, 3]),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap();
		assert!(output.pass);
	}

	#[test]
	fn test_rhai_context_access() {
		let checker = RhaiChecker::new("result == context.answer");
		let output = checker
			.check(&CheckInput {
				result: json!(42),
				expected: json!(null),
				context: json!({"answer": 42}),
			})
			.unwrap();
		assert!(output.pass);
	}

	#[test]
	fn test_rhai_syntax_error() {
		let checker = RhaiChecker::new("invalid $$$ syntax");
		let output = checker
			.check(&CheckInput {
				result: json!(1),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap_err();
		assert!(output.message.contains("could not evaluate"));
	}

	#[test]
	fn test_rhai_non_bool_return() {
		let checker = RhaiChecker::new("result + 1");
		let output = checker
			.check(&CheckInput {
				result: json!(5),
				expected: json!(null),
				context: json!({}),
			})
			.unwrap_err();
		assert!(output.message.contains("not a bool"));
	}
}
