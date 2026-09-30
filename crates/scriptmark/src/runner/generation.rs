//! A template's concrete cases: its samples as written, then its random draws.
//!
//! `generate` is pure: it is given the seed, never draws one, so the same template and
//! seed always give the same cases, whatever else is being prepared alongside.

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::SeedableRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{Inputs, Seed, TestSpec};
use crate::runner::generator::{GENERATOR_VERSION, Rule};

/// The most draws one template may make.
pub const MAX_COUNT: usize = 10_000;

/// The most values or characters one template's draws may add up to, at worst.
pub const MAX_GENERATED: u64 = 1_000_000;

/// Where a concrete case came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
	Sample(usize),
	Draw(usize),
}

/// How the seed a template drew with was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedSource {
	/// None was declared: 0.
	Default,
	/// `seed = N`.
	Declared,
	/// `seed = "random"`.
	Drawn,
}

/// Draws a seed for `seed = "random"`: the OS, or a test's stand-in.
pub type DrawSeed = fn() -> Result<u64, String>;

/// One concrete case of a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Concrete {
	pub name: String,
	pub origin: Origin,
	/// The arguments, in call order.
	pub args: Vec<Value>,
}

/// A template's inputs, and what it takes to make them again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generated {
	/// The `GENERATOR_VERSION` that drew these inputs.
	pub generator: u32,
	/// The settings, as the spec wrote them.
	pub inputs: Inputs,
	/// The seed the draws used; `None` when nothing was random.
	pub seed: Option<u64>,
	pub seed_source: Option<SeedSource>,
	/// Samples first, then draws.
	pub cases: Vec<Concrete>,
}

/// The name a concrete case is reported under.
pub fn concrete_name(case: &str, origin: Origin) -> String {
	match origin {
		Origin::Sample(j) => format!("{case} [sample {j}]"),
		Origin::Draw(i) => format!("{case} [{i}]"),
	}
}

/// Where each concrete case of a template comes from, in order: samples, then draws.
pub fn origins(inputs: &Inputs) -> impl Iterator<Item = Origin> {
	let draws = inputs.random.as_ref().map_or(0, |r| r.count);
	(0..inputs.samples.len())
		.map(Origin::Sample)
		.chain((0..draws).map(Origin::Draw))
}

/// The seed a template draws with, and how it was chosen. `None` when nothing is random:
/// no draws, or no parameters to draw.
pub fn seed_for(inputs: &Inputs, draw: DrawSeed) -> Result<Option<(u64, SeedSource)>, String> {
	let Some(random) = inputs.random.as_ref().filter(|_| !inputs.args.is_empty()) else {
		return Ok(None);
	};
	Ok(Some(match random.seed {
		None => (0, SeedSource::Default),
		Some(Seed::Fixed(seed)) => (seed, SeedSource::Declared),
		Some(Seed::Random) => (draw()?, SeedSource::Drawn),
	}))
}

/// The first template, as `(spec, case)`, whose seed would be drawn: `seed = "random"` on
/// a template that draws from rules.
pub fn draws_a_seed(specs: &[TestSpec]) -> Option<(&str, &str)> {
	specs.iter().find_map(|spec| {
		spec.cases.iter().find_map(|case| {
			let inputs = case.parametrize.as_ref()?.inputs();
			let random = inputs.random.as_ref().filter(|_| !inputs.args.is_empty())?;
			(random.seed == Some(Seed::Random))
				.then_some((spec.meta.name.as_str(), case.name.as_str()))
		})
	})
}

/// A seed from the OS, kept below 2^53 so it fits a TOML integer and a JSON reader's
/// double exactly.
pub fn os_seed() -> Result<u64, String> {
	use rand::TryRngCore;
	rand::rngs::OsRng
		.try_next_u64()
		.map(|seed| seed & ((1 << 53) - 1))
		.map_err(|e| format!("could not draw a random seed: {e}"))
}

/// The concrete cases of one template. Draw `i` reads stream `i` of the seed, one value per
/// parameter in call order, so more draws never change the earlier ones.
pub fn generate(
	case: &str,
	inputs: &Inputs,
	seed: Option<(u64, SeedSource)>,
) -> Result<Generated, Vec<String>> {
	let mut rules = Vec::new();
	let mut problems = Vec::new();
	for param in &inputs.args {
		match Rule::parse(&param.rule) {
			Ok(rule) => rules.push(rule),
			Err(e) => problems.push(format!("parameter '{}': {e}", param.name)),
		}
	}
	if !problems.is_empty() {
		return Err(problems);
	}

	let cases = origins(inputs)
		.map(|origin| Concrete {
			name: concrete_name(case, origin),
			origin,
			args: match origin {
				Origin::Sample(j) => inputs.samples[j].clone(),
				Origin::Draw(i) => {
					let mut rng = ChaCha8Rng::seed_from_u64(seed.map_or(0, |(s, _)| s));
					rng.set_stream(i as u64);
					rules.iter().map(|rule| rule.draw(&mut rng)).collect()
				}
			},
		})
		.collect();
	Ok(Generated {
		generator: GENERATOR_VERSION,
		inputs: inputs.clone(),
		seed: seed.map(|(s, _)| s),
		seed_source: seed.map(|(_, source)| source),
		cases,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{Param, Random};
	use serde_json::json;

	fn inputs(
		args: &[(&str, &str)],
		samples: Vec<Vec<Value>>,
		random: Option<(usize, Option<Seed>)>,
	) -> Inputs {
		Inputs {
			args: args
				.iter()
				.map(|(name, rule)| Param {
					name: name.to_string(),
					rule: rule.to_string(),
				})
				.collect(),
			samples,
			random: random.map(|(count, seed)| Random { count, seed }),
		}
	}

	fn declared(seed: u64) -> Option<(u64, SeedSource)> {
		Some((seed, SeedSource::Declared))
	}

	fn args_of(generated: &Generated) -> Vec<Vec<Value>> {
		generated.cases.iter().map(|c| c.args.clone()).collect()
	}

	#[test]
	fn test_names_say_where_a_case_came_from() {
		assert_eq!(
			concrete_name("clamp", Origin::Sample(0)),
			"clamp [sample 0]"
		);
		assert_eq!(concrete_name("clamp", Origin::Draw(3)), "clamp [3]");
	}

	#[test]
	fn test_samples_come_first_then_draws_in_order() {
		let t = inputs(
			&[("x", "int(0, 9)")],
			vec![vec![json!(-1)], vec![json!(99)]],
			Some((2, None)),
		);
		let g = generate("t", &t, declared(1)).unwrap();
		let names: Vec<&str> = g.cases.iter().map(|c| c.name.as_str()).collect();
		assert_eq!(names, ["t [sample 0]", "t [sample 1]", "t [0]", "t [1]"]);
		let origins: Vec<Origin> = g.cases.iter().map(|c| c.origin).collect();
		assert_eq!(
			origins,
			[
				Origin::Sample(0),
				Origin::Sample(1),
				Origin::Draw(0),
				Origin::Draw(1)
			]
		);
		assert_eq!(g.cases[0].args, [json!(-1)]);
		assert_eq!(g.cases[1].args, [json!(99)]);
		assert_eq!(g.generator, GENERATOR_VERSION);
		assert_eq!(g.inputs, t);
	}

	#[test]
	fn test_arguments_are_drawn_in_call_order() {
		let t = inputs(
			&[
				("x", "int(0, 9)"),
				("lo", "int(100, 109)"),
				("hi", "int(1000, 1009)"),
			],
			vec![],
			Some((50, None)),
		);
		for args in args_of(&generate("t", &t, declared(5)).unwrap()) {
			let n: Vec<i64> = args.iter().map(|v| v.as_i64().unwrap()).collect();
			assert!(
				(0..=9).contains(&n[0])
					&& (100..=109).contains(&n[1])
					&& (1000..=1009).contains(&n[2]),
				"{n:?}"
			);
		}
	}

	#[test]
	fn test_the_same_seed_gives_the_same_cases() {
		let t = inputs(
			&[("a", "int(0, 1000000)"), ("s", "str(0, 5)")],
			vec![],
			Some((20, None)),
		);
		assert_eq!(
			generate("t", &t, declared(9)),
			generate("t", &t, declared(9))
		);
		assert_ne!(
			args_of(&generate("t", &t, declared(9)).unwrap()),
			args_of(&generate("t", &t, declared(10)).unwrap())
		);
	}

	#[test]
	fn test_more_draws_or_samples_keep_the_earlier_draws() {
		let draws = |samples: usize, count: usize| {
			let t = inputs(
				&[("a", "int(0, 1000000)")],
				vec![vec![json!(0)]; samples],
				Some((count, None)),
			);
			let g = generate("t", &t, declared(3)).unwrap();
			g.cases
				.into_iter()
				.filter(|c| matches!(c.origin, Origin::Draw(_)))
				.map(|c| c.args)
				.collect::<Vec<_>>()
		};
		let few = draws(0, 5);
		assert_eq!(draws(0, 50)[..5], few[..]);
		assert_eq!(draws(4, 5), few);
	}

	#[test]
	fn test_a_template_with_no_parameters_calls_count_times() {
		let t = inputs(&[], vec![], Some((3, None)));
		let g = generate("t", &t, None).unwrap();
		assert_eq!(args_of(&g), vec![Vec::<Value>::new(); 3]);
		assert_eq!((g.seed, g.seed_source), (None, None));
	}

	#[test]
	fn test_a_bad_rule_is_an_error_not_a_null() {
		let t = inputs(
			&[("a", "int(5, 1)"), ("b", "nope()")],
			vec![],
			Some((1, None)),
		);
		let problems = generate("t", &t, declared(0)).unwrap_err();
		assert_eq!(problems.len(), 2, "{problems:?}");
		assert!(
			problems[0].contains("parameter 'a'") && problems[0].contains("greater than"),
			"{problems:?}"
		);
		assert!(
			problems[1].contains("parameter 'b'") && problems[1].contains("unknown rule"),
			"{problems:?}"
		);
	}

	#[test]
	fn test_the_seed_is_chosen_only_when_something_is_random() {
		fn never() -> Result<u64, String> {
			panic!("no seed should be drawn")
		}
		fn four() -> Result<u64, String> {
			Ok(4)
		}
		fn broken() -> Result<u64, String> {
			Err("no entropy".into())
		}
		let with = |seed| inputs(&[("a", "int(0, 1)")], vec![], Some((1, seed)));
		assert_eq!(
			seed_for(&with(None), never),
			Ok(Some((0, SeedSource::Default)))
		);
		assert_eq!(
			seed_for(&with(Some(Seed::Fixed(7))), never),
			Ok(Some((7, SeedSource::Declared)))
		);
		assert_eq!(
			seed_for(&with(Some(Seed::Random)), four),
			Ok(Some((4, SeedSource::Drawn)))
		);
		assert_eq!(
			seed_for(&with(Some(Seed::Random)), broken),
			Err("no entropy".into())
		);
		// Nothing drawn, or nothing to draw: no seed at all.
		let samples_only = inputs(&[("a", "int(0, 1)")], vec![vec![json!(1)]], None);
		assert_eq!(seed_for(&samples_only, never), Ok(None));
		assert_eq!(
			seed_for(&inputs(&[], vec![], Some((2, None))), never),
			Ok(None)
		);
	}

	#[test]
	fn test_only_a_random_seed_on_a_template_that_draws_is_drawn() {
		let spec = |body: &str| {
			crate::spec_loader::load_spec_str(
				&format!("[meta]\nname = \"s\"\nfile = \"l.py\"\nfunction = \"f\"\nlanguage = \"python\"\n[[cases]]\nname = \"c\"\ncheck = \"sorted\"\n{body}"),
				std::path::Path::new("."),
			)
			.unwrap_or_else(|e| panic!("{e}"))
		};
		let one = "[[cases.parametrize.args]]\na = \"list(int(0, 1), 0, 2)\"\n";
		let random = format!("{one}[cases.parametrize.random]\ncount = 1\nseed = \"random\"\n");
		assert_eq!(draws_a_seed(&[spec(&random)]), Some(("s", "c")));
		for quiet in [
			format!("{one}[cases.parametrize.random]\ncount = 1\nseed = 4\n"),
			format!("{one}[cases.parametrize.random]\ncount = 1\n"),
			format!("{one}[cases.parametrize]\nsamples = [[[1]]]\n"),
		] {
			assert_eq!(draws_a_seed(&[spec(&quiet)]), None, "{quiet}");
		}
	}

	#[test]
	fn test_a_drawn_seed_fits_toml_and_json() {
		for _ in 0..100 {
			assert!(os_seed().unwrap() < 1 << 53);
		}
	}

	/// Pins seed → streams → call order → names. A change here changes the inputs of every
	/// seeded template: bump `GENERATOR_VERSION` and update these values together.
	#[test]
	fn test_golden_generate_v1() {
		let t = inputs(
			&[
				("value", "choice([-100, -75, -25, 0, 25, 75, 100])"),
				("low", "int(-49, -26)"),
				("high", "int(26, 49)"),
			],
			vec![
				vec![json!(-30), json!(-30), json!(30)],
				vec![json!(30), json!(-30), json!(30)],
			],
			Some((3, Some(Seed::Fixed(42)))),
		);
		let g = generate("clamp", &t, declared(42)).unwrap();
		let got: Vec<(String, Vec<Value>)> =
			g.cases.into_iter().map(|c| (c.name, c.args)).collect();
		let row = |name: &str, args: [i64; 3]| (name.to_string(), args.map(Value::from).to_vec());
		assert_eq!(
			got,
			[
				row("clamp [sample 0]", [-30, -30, 30]),
				row("clamp [sample 1]", [30, -30, 30]),
				row("clamp [0]", [75, -33, 30]),
				row("clamp [1]", [100, -28, 49]),
				row("clamp [2]", [100, -47, 35]),
			]
		);
	}
}
