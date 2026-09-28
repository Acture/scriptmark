//! Expected values for generated cases. P-676 owns what an answer source is and how it
//! is frozen; this resolves the oracles a spec already declares, once per bundle.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::models::spec::Oracle;
use crate::models::{Target, TestCase, TestSpec};
use crate::runner::executor::{CallPlan, Executor, Outcome, Subject, UnitPlan};

/// Fill in `case`'s expectation from its oracle. A reference that does not return, or a
/// Rhai expression that yields nothing usable, is an error — never a null expectation.
pub async fn resolve_oracle<E: Executor>(
	case: &mut TestCase,
	oracle: &Oracle,
	spec: &TestSpec,
	executor: &E,
	arg_names: &[String],
	timeout_secs: u64,
) -> Result<(), String> {
	if let Some(reference) = &oracle.reference {
		let name = case
			.function
			.clone()
			.or_else(|| spec.meta.function.clone())
			.ok_or("a reference oracle needs a function to call")?;
		let plan = UnitPlan {
			subject: Subject::Reference,
			file: reference.into(),
			script: None,
			imports: Vec::new(),
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
				stdin: None,
				timeout: timeout_secs,
				id: None,
				files: Vec::new(),
				check: None,
			}],
		};
		let obs = executor.run(&plan).await;
		return match obs.steps.first().map(|c| &c.outcome) {
			Some(Outcome::Returned { value, .. }) => {
				case.expect = Some(value.clone());
				Ok(())
			}
			Some(other) => Err(format!(
				"reference implementation '{name}' did not return a value: {other:?}"
			)),
			None => Err(format!(
				"reference implementation '{reference}' did not run ({:?}{})",
				obs.exit,
				obs.load
					.as_ref()
					.map(|l| format!(", load: {:?}", l.outcome))
					.unwrap_or_default()
			)),
		};
	}
	if let Some(expr) = &oracle.rhai {
		let engine = rhai::Engine::new();
		let mut scope = rhai::Scope::new();
		for (name, value) in arg_names.iter().zip(&case.args) {
			scope.push_dynamic(
				name.as_str(),
				crate::checker::rhai_checker::json_to_dynamic(value),
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
