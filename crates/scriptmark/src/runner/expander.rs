use crate::models::TestCase;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::runner::generator::generate_value;

/// Expand parametrized TestCases into concrete TestCases.
/// Non-parametrized cases pass through unchanged.
pub fn expand_cases(cases: &[TestCase]) -> Vec<TestCase> {
	cases.iter().flat_map(expand_case).collect()
}

/// The concrete cases one case stands for: itself, or its generated cases. A generated
/// case keeps everything but `parametrize` — its target, checks and timeout included.
pub fn expand_case(case: &TestCase) -> Vec<TestCase> {
	let Some(param) = &case.parametrize else {
		return vec![case.clone()];
	};
	let mut rng = StdRng::seed_from_u64(param.seed.unwrap_or(0));
	// `args` is a BTreeMap, so arguments bind in alphabetical order (P-675 owns binding).
	(0..param.count)
		.map(|i| TestCase {
			name: format!("{} [{}]", case.name, i),
			args: param
				.args
				.values()
				.map(|expr| generate_value(expr, &mut rng).unwrap_or(serde_json::Value::Null))
				.collect(),
			parametrize: None,
			..case.clone()
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::spec::{Oracle, Parametrize};
	use std::collections::BTreeMap;

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
		let mut args = BTreeMap::new();
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
		let mut args = BTreeMap::new();
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

	#[test]
	fn test_a_generated_case_keeps_its_target() {
		let case = TestCase {
			name: "g".into(),
			function: Some("other".into()),
			timeout: Some(3),
			parametrize: Some(Parametrize {
				count: 2,
				seed: None,
				args: BTreeMap::from([("x".into(), "int(0, 1)".into())]),
				oracle: Oracle::default(),
			}),
			..Default::default()
		};
		for generated in expand_case(&case) {
			assert_eq!(generated.function.as_deref(), Some("other"));
			assert_eq!(generated.timeout, Some(3));
		}
	}
}
