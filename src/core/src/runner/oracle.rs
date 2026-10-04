//! Resolve a teacher's answer source before any student runs.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::models::spec::Oracle;
use crate::models::{Target, TestCase, TestSpec};
use crate::runner::executor::{CallPlan, Executor, Exit, Outcome, Subject, UnitPlan};

/// Fill the declared observations. Unexpected reference failures and unusable Rhai
/// values are preparation errors; None is a return expectation only when explicit.
pub async fn resolve_oracle<E: Executor>(
	case: &mut TestCase,
	oracle: &Oracle,
	spec: &TestSpec,
	executor: &E,
	arg_names: &[String],
	timeout_secs: u64,
) -> Result<(), String> {
	if let Some(reference) = &oracle.reference {
		let name = oracle
			.function
			.as_ref()
			.ok_or("a reference oracle needs an explicit function")?;
		let timeout_secs = case.timeout.unwrap_or(timeout_secs);
		let plan = UnitPlan {
			subject: Subject::Reference,
			functions: crate::matching::Functions::default(),
			file: reference.into(),
			script: None,
			imports: spec.meta.imports.clone(),
			vars: Arc::new(spec.vars.clone()),
			data_files: spec
				.meta
				.data_files
				.iter()
				.map(|rel| (spec.dir.join(rel), rel.clone()))
				.collect(),
			allowed_imports: spec.meta.allowed_imports.clone(),
			load_timeout: timeout_secs,
			setup: Vec::new(),
			steps: vec![CallPlan {
				target: Target::Function { name: name.clone() },
				args: case.args.clone(),
				stdin: case.stdin.clone(),
				timeout: timeout_secs,
				id: None,
				files: oracle.files.clone(),
				check: None,
			}],
		};
		let obs = executor.run(&plan).await;
		if !obs.ready
			|| !obs.done
			|| obs.exit != Exit::Code(0)
			|| obs.fatal.is_some()
			|| obs.protocol_error.is_some()
		{
			return Err(format!(
				"reference implementation '{reference}' did not finish reliably: exit={:?}, fatal={:?}, protocol={:?}",
				obs.exit, obs.fatal, obs.protocol_error
			));
		}
		let call = obs.steps.first().ok_or_else(|| {
			format!(
				"reference implementation '{reference}' did not run (load: {:?})",
				obs.load
			)
		})?;
		match (&call.outcome, &oracle.raises) {
			(Outcome::Returned { value, .. }, None) => {
				if oracle.returns_value() {
					if value.is_null() && oracle.returns != Some(true) {
						return Err(format!(
							"reference implementation '{name}' returned None; declare returns = true to expect None, or returns = false with stdout/files"
						));
					}
					case.expect = Some(value.clone());
				}
			}
			(Outcome::Raised(error), Some(expected)) if error.is_a(expected) => {
				case.expect_error = Some(expected.clone());
			}
			(other, _) => {
				return Err(format!(
					"reference implementation '{name}' produced an unexpected outcome: {other:?}; declared raises={:?}",
					oracle.raises
				));
			}
		}
		if oracle.stdout {
			if call.stdout_truncated {
				return Err(format!(
					"reference implementation '{name}' stdout was truncated"
				));
			}
			case.expected_stdout = Some(call.stdout.clone());
		}
		for path in &oracle.files {
			let content = call
				.files
				.get(path)
				.and_then(Option::as_ref)
				.ok_or_else(|| {
					format!(
						"reference implementation '{name}' did not produce readable text file '{path}'"
					)
				})?;
			case.expect_files.insert(path.clone(), content.clone());
		}
		return Ok(());
	}
	if let Some(expr) = &oracle.rhai {
		let engine = crate::checker::rhai_checker::engine();
		let mut scope = rhai::Scope::new();
		if arg_names.is_empty() {
			scope.push_dynamic(
				"args",
				crate::checker::rhai_checker::json_to_dynamic(&literal(&serde_json::Value::from(
					case.args.clone(),
				))),
			);
		}
		for (name, value) in arg_names.iter().zip(&case.args) {
			scope.push_dynamic(
				name.as_str(),
				crate::checker::rhai_checker::json_to_dynamic(&literal(value)),
			);
		}
		let result = engine
			.eval_with_scope::<rhai::Dynamic>(&mut scope, expr)
			.map_err(|e| format!("rhai oracle `{expr}` failed: {e}"))?;
		case.expect = Some(dynamic_to_json(&result).ok_or_else(|| {
			format!(
				"rhai oracle `{expr}` returned {}, not a value",
				result.type_name()
			)
		})?);
		return Ok(());
	}
	if let Some(name) = &oracle.check {
		case.check = Some(crate::models::CheckMethod::Builtin(name.clone()));
	}
	Ok(())
}

/// Check argument count for every reference call and parameter names for templates.
/// Inspect each file once; this never calls the reference entry point.
pub async fn validate_references<E: Executor>(
	spec: &TestSpec,
	arg_names: &BTreeMap<String, Vec<String>>,
	executor: &E,
	timeout_secs: u64,
) -> Result<(), String> {
	let mut inspected: BTreeMap<String, crate::runner::executor::TeacherRuntime> = BTreeMap::new();
	for case in &spec.cases {
		let Some(oracle) = case.oracle.as_ref().filter(|o| o.reference.is_some()) else {
			continue;
		};
		let file = oracle.reference.as_ref().expect("reference was checked");
		if !inspected.contains_key(file) {
			let mut reference: TestSpec = spec.clone();
			reference.meta.imports = vec![file.clone()];
			let runtime = executor
				.inspect(&reference, timeout_secs)
				.await
				.map_err(|e| format!("reference '{file}': {e}"))?;
			inspected.insert(file.clone(), runtime);
		}
		let name = oracle.function.as_ref().expect("validated reference entry");
		let params = inspected[file]
			.exports
			.get(name)
			.filter(|e| e.callable)
			.and_then(|e| e.params.as_ref())
			.ok_or_else(|| {
				format!("reference '{file}' has no inspectable public function '{name}'")
			})?;
		let positional: Vec<&crate::runner::executor::Param> = params
			.iter()
			.filter(|p| p.kind == "positional_only" || p.kind == "positional_or_keyword")
			.collect();
		let count = case.args.len();
		let required = positional.iter().filter(|p| !p.default).count();
		let variadic = params.iter().any(|p| p.kind == "var_positional");
		if count < required
			|| (count > positional.len() && !variadic)
			|| params
				.iter()
				.any(|p| p.kind == "keyword_only" && !p.default)
		{
			return Err(format!(
				"case '{}': reference '{name}' signature cannot accept {count} positional arguments",
				case.name
			));
		}
		if let Some(names) = arg_names.get(&case.name) {
			let expected: Vec<&str> = positional
				.iter()
				.take(count)
				.map(|p| p.name.as_str())
				.collect();
			let actual: Vec<&str> = names.iter().map(String::as_str).collect();
			if expected != actual {
				return Err(format!(
					"case '{}': template parameters {actual:?} do not match reference '{name}' signature {expected:?} in call order",
					case.name
				));
			}
		}
	}
	Ok(())
}

/// A value as the student receives it: the harness turns each `$$…` string into `$…`, in
/// lists and dict values alike. A generated value holds no `$name` reference.
pub fn literal(value: &serde_json::Value) -> serde_json::Value {
	use serde_json::Value;
	match value {
		Value::String(s) if s.starts_with("$$") => Value::String(s[1..].to_string()),
		Value::Array(items) => Value::Array(items.iter().map(literal).collect()),
		Value::Object(map) => {
			Value::Object(map.iter().map(|(k, v)| (k.clone(), literal(v))).collect())
		}
		other => other.clone(),
	}
}

/// A Rhai value as JSON, arrays and maps included. `None` for `()` and anything else
/// with no JSON form.
fn dynamic_to_json(val: &rhai::Dynamic) -> Option<serde_json::Value> {
	use serde_json::Value;
	if val.is_unit() {
		return None;
	}
	if let Ok(b) = val.as_bool() {
		return Some(Value::from(b));
	}
	if let Ok(i) = val.as_int() {
		return Some(Value::from(i));
	}
	if let Ok(f) = val.as_float() {
		return serde_json::Number::from_f64(f).map(Value::Number);
	}
	if val.is_string() {
		return val.clone().into_string().ok().map(Value::from);
	}
	if let Some(items) = val.clone().try_cast::<rhai::Array>() {
		return items
			.iter()
			.map(dynamic_to_json)
			.collect::<Option<Vec<_>>>()
			.map(Value::Array);
	}
	if let Some(map) = val.clone().try_cast::<rhai::Map>() {
		return map
			.iter()
			.map(|(k, v)| dynamic_to_json(v).map(|v| (k.to_string(), v)))
			.collect::<Option<BTreeMap<_, _>>>()
			.map(|m| Value::Object(m.into_iter().collect()));
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn eval(expr: &str) -> Option<serde_json::Value> {
		dynamic_to_json(&rhai::Engine::new().eval::<rhai::Dynamic>(expr).unwrap())
	}

	#[test]
	fn test_a_literal_is_what_the_harness_hands_the_student() {
		assert_eq!(literal(&json!("$$5")), json!("$5"));
		assert_eq!(literal(&json!("$$$x")), json!("$$x"));
		assert_eq!(literal(&json!("a$$")), json!("a$$"));
		assert_eq!(
			literal(&json!([["$$a"], {"$$k": "$$v"}, 3])),
			json!([["$a"], {"$$k": "$v"}, 3]),
			"dict keys are left alone, as the harness leaves them"
		);
	}

	#[test]
	fn test_rhai_values_convert_totally() {
		assert_eq!(
			eval("[1, 2.5, \"x\", true]"),
			Some(json!([1, 2.5, "x", true]))
		);
		assert_eq!(eval("#{a: [1]}"), Some(json!({"a": [1]})));
		assert_eq!(eval("()"), None, "unit is no expectation");
		assert_eq!(eval("[1, ()]"), None);
	}
}
