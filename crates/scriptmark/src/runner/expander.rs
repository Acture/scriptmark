use crate::models::TestCase;
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::SeedableRng;

use crate::runner::generator::Rule;

/// The concrete cases one case stands for: itself, or its generated cases. A generated
/// case keeps everything but `parametrize` — its target, checks and timeout included.
pub fn expand_case(case: &TestCase) -> Vec<TestCase> {
	let Some(param) = &case.parametrize else {
		return vec![case.clone()];
	};
	let mut rng = ChaCha8Rng::seed_from_u64(param.seed.unwrap_or(0));
	// `args` is a BTreeMap, so arguments bind in alphabetical order (P-675 owns binding).
	(0..param.count)
		.map(|i| TestCase {
			name: format!("{} [{}]", case.name, i),
			args: param
				.args
				.values()
				.map(|expr| {
					Rule::parse(expr)
						.map(|rule| rule.draw(&mut rng))
						.unwrap_or(serde_json::Value::Null)
				})
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

	fn parametrized(name: &str, count: usize, seed: u64, args: &[(&str, &str)]) -> TestCase {
		TestCase {
			name: name.into(),
			parametrize: Some(Parametrize {
				count,
				seed: Some(seed),
				args: args
					.iter()
					.map(|(k, v)| (k.to_string(), v.to_string()))
					.collect(),
				oracle: Oracle::default(),
			}),
			..Default::default()
		}
	}

	#[test]
	fn test_non_parametrized_passthrough() {
		let case = TestCase {
			name: "simple".into(),
			args: vec![serde_json::json!(3), serde_json::json!(5)],
			expect: Some(serde_json::json!(5)),
			..Default::default()
		};
		let expanded = expand_case(&case);
		assert_eq!(expanded.len(), 1);
		assert_eq!(expanded[0].name, "simple");
	}

	#[test]
	fn test_parametrized_expansion() {
		let case = parametrized(
			"random test",
			5,
			42,
			&[("a", "int(0, 10)"), ("b", "int(0, 10)")],
		);
		let expanded = expand_case(&case);
		assert_eq!(expanded.len(), 5);
		for (i, case) in expanded.iter().enumerate() {
			assert_eq!(case.name, format!("random test [{i}]"));
			assert_eq!(case.args.len(), 2);
			assert!(case.parametrize.is_none());
		}
	}

	#[test]
	fn test_seed_reproducibility() {
		let case = parametrized("seeded", 3, 99, &[("x", "int(0, 1000)")]);
		let (run1, run2) = (expand_case(&case), expand_case(&case));
		assert_eq!(run1[0].args, run2[0].args);
		assert_eq!(run1[1].args, run2[1].args);
	}

	#[test]
	fn test_a_generated_case_keeps_its_target() {
		let case = TestCase {
			function: Some("other".into()),
			timeout: Some(3),
			..parametrized("g", 2, 0, &[("x", "int(0, 1)")])
		};
		for generated in expand_case(&case) {
			assert_eq!(generated.function.as_deref(), Some("other"));
			assert_eq!(generated.timeout, Some(3));
		}
	}
}
