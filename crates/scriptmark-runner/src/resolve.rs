use std::collections::HashMap;

use serde_json::Value;

/// Resolve `$ref` references in a JSON value tree.
///
/// Any string starting with `$` is looked up in the context map.
/// For example, `"$data"` is replaced with `context["data"]`.
pub fn resolve_refs(value: &Value, context: &HashMap<String, Value>) -> Value {
	match value {
		Value::String(s) if s.starts_with('$') => {
			let key = &s[1..];
			context.get(key).cloned().unwrap_or(Value::Null)
		}
		Value::Array(arr) => Value::Array(arr.iter().map(|v| resolve_refs(v, context)).collect()),
		Value::Object(obj) => Value::Object(
			obj.iter()
				.map(|(k, v)| (k.clone(), resolve_refs(v, context)))
				.collect(),
		),
		other => other.clone(),
	}
}

/// Resolve `$ref` in a list of args.
pub fn resolve_args(args: &[Value], context: &HashMap<String, Value>) -> Vec<Value> {
	args.iter().map(|v| resolve_refs(v, context)).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn test_resolve_string_ref() {
		let mut ctx = HashMap::new();
		ctx.insert("data".to_string(), json!({"stations": [1, 2, 3]}));

		let resolved = resolve_refs(&json!("$data"), &ctx);
		assert_eq!(resolved, json!({"stations": [1, 2, 3]}));
	}

	#[test]
	fn test_resolve_in_array() {
		let mut ctx = HashMap::new();
		ctx.insert("x".to_string(), json!(42));

		let resolved = resolve_args(&[json!("$x"), json!(1), json!("hello")], &ctx);
		assert_eq!(resolved, vec![json!(42), json!(1), json!("hello")]);
	}

	#[test]
	fn test_resolve_missing_ref() {
		let ctx = HashMap::new();
		let resolved = resolve_refs(&json!("$missing"), &ctx);
		assert_eq!(resolved, Value::Null);
	}

	#[test]
	fn test_no_ref_passthrough() {
		let ctx = HashMap::new();
		let resolved = resolve_refs(&json!("hello"), &ctx);
		assert_eq!(resolved, json!("hello"));
	}

	#[test]
	fn test_resolve_nested_object() {
		let mut ctx = HashMap::new();
		ctx.insert("neighbors".to_string(), json!([1, 2, 3]));

		let value = json!({"data": "$neighbors", "other": 5});
		let resolved = resolve_refs(&value, &ctx);
		assert_eq!(resolved, json!({"data": [1, 2, 3], "other": 5}));
	}
}
