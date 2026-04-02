use rand::SeedableRng;
use rand::rngs::StdRng;
use scriptmark_core::models::TestCase;

use crate::generator::generate_value;

/// Expand parametrized TestCases into concrete TestCases.
/// Non-parametrized cases pass through unchanged.
pub fn expand_cases(cases: &[TestCase]) -> Vec<TestCase> {
	let mut result = Vec::new();

	for case in cases {
		if let Some(param) = &case.parametrize {
			let seed = param.seed.unwrap_or(0);
			let mut rng = StdRng::seed_from_u64(seed);

			// Sort arg names for deterministic positional ordering
			let mut arg_names: Vec<&String> = param.args.keys().collect();
			arg_names.sort();

			for i in 0..param.count {
				let mut args = Vec::new();
				for name in &arg_names {
					let expr = &param.args[*name];
					match generate_value(expr, &mut rng) {
						Ok(val) => args.push(val),
						Err(_) => args.push(serde_json::Value::Null),
					}
				}

				result.push(TestCase {
					name: format!("{} [{}]", case.name, i),
					args,
					check: case.check.clone(),
					timeout: case.timeout,
					..Default::default()
				});
			}
		} else {
			result.push(case.clone());
		}
	}

	result
}

#[cfg(test)]
mod tests {
	use super::*;
	use scriptmark_core::models::spec::{Oracle, Parametrize};
	use std::collections::HashMap;

	#[test]
	fn test_non_parametrized_passthrough() {
		let cases = vec![TestCase {
			name: "simple".into(),
			args: vec![serde_json::json!(3), serde_json::json!(5)],
			expect: Some(serde_json::json!(5)),
			..Default::default()
		}];
		let expanded = expand_cases(&cases);
		assert_eq!(expanded.len(), 1);
		assert_eq!(expanded[0].name, "simple");
	}

	#[test]
	fn test_parametrized_expansion() {
		let mut args = HashMap::new();
		args.insert("a".into(), "int(0, 10)".into());
		args.insert("b".into(), "int(0, 10)".into());

		let cases = vec![TestCase {
			name: "random test".into(),
			parametrize: Some(Parametrize {
				count: 5,
				seed: Some(42),
				args,
				oracle: Oracle::default(),
			}),
			..Default::default()
		}];
		let expanded = expand_cases(&cases);
		assert_eq!(expanded.len(), 5);
		for (i, case) in expanded.iter().enumerate() {
			assert_eq!(case.name, format!("random test [{}]", i));
			assert_eq!(case.args.len(), 2);
			assert!(case.parametrize.is_none());
		}
	}

	#[test]
	fn test_seed_reproducibility() {
		let mut args = HashMap::new();
		args.insert("x".into(), "int(0, 1000)".into());

		let cases = vec![TestCase {
			name: "seeded".into(),
			parametrize: Some(Parametrize {
				count: 3,
				seed: Some(99),
				args,
				oracle: Oracle::default(),
			}),
			..Default::default()
		}];
		let run1 = expand_cases(&cases);
		let run2 = expand_cases(&cases);
		assert_eq!(run1[0].args, run2[0].args);
		assert_eq!(run1[1].args, run2[1].args);
	}
}
