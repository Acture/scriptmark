//! Frozen inputs: every template's concrete cases for one batch, written beside its results
//! so that a later run — a regrade, late submissions — can use exactly the same inputs.
//!
//! The file holds inputs only. What the right answer is stays with the oracles (P-676).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{Inputs, Seed};
use crate::runner::generation::{
	DrawSeed, Generated, MAX_COUNT, Origin, concrete_name, origins, os_seed,
};
use crate::runner::prepare::Bundle;

/// The layout of the file. A newer one is refused by name, never half-read.
pub const FORMAT: u32 = 1;

/// Every template's inputs, by spec name, then template name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
	pub format: u32,
	/// The build that wrote the file.
	pub scriptmark: String,
	pub specs: BTreeMap<String, BTreeMap<String, Generated>>,
}

/// Where a batch's inputs come from.
#[derive(Debug, Clone)]
pub enum Generation {
	/// Made now; `seed = "random"` draws from this.
	Fresh(DrawSeed),
	/// Taken as they are from frozen inputs.
	Replay(Frozen),
}

impl Generation {
	pub fn fresh() -> Self {
		Generation::Fresh(os_seed)
	}
}

#[derive(Debug, thiserror::Error)]
pub enum FrozenError {
	#[error("cannot read frozen inputs {path}: {error}", path = .0.display(), error = .1)]
	Io(PathBuf, std::io::Error),
	#[error("{path} is not frozen inputs this build can read: {why}", path = .0.display(), why = .1)]
	Invalid(PathBuf, String),
}

impl Frozen {
	/// The inputs the bundles were prepared with.
	pub fn of(bundles: &[Bundle]) -> Frozen {
		Frozen {
			format: FORMAT,
			scriptmark: env!("CARGO_PKG_VERSION").to_string(),
			specs: bundles
				.iter()
				.filter(|b| !b.generated.is_empty())
				.map(|b| (b.spec.meta.name.clone(), b.generated.clone()))
				.collect(),
		}
	}

	/// Whether no template made any inputs.
	pub fn is_empty(&self) -> bool {
		self.specs.values().all(BTreeMap::is_empty)
	}

	pub fn to_json(&self) -> String {
		serde_json::to_string_pretty(self).expect("frozen inputs are plain JSON") + "\n"
	}

	/// The header first, then the strict body, so a newer format is refused by its number
	/// rather than by a field this build has never heard of.
	pub fn from_json(text: &str) -> Result<Frozen, String> {
		#[derive(Deserialize)]
		struct Header {
			format: Option<u32>,
		}
		let header: Header = serde_json::from_str(text).map_err(|e| e.to_string())?;
		match header.format {
			None => Err("it has no format".into()),
			Some(FORMAT) => serde_json::from_str(text).map_err(|e| e.to_string()),
			Some(other) => Err(format!(
				"it is format {other}, and this build reads format {FORMAT}"
			)),
		}
	}

	pub fn load(path: &Path) -> Result<Frozen, FrozenError> {
		let text =
			std::fs::read_to_string(path).map_err(|e| FrozenError::Io(path.to_path_buf(), e))?;
		Frozen::from_json(&text).map_err(|e| FrozenError::Invalid(path.to_path_buf(), e))
	}

	/// Write through a temporary file and a rename, so the file is never half-written,
	/// creating its directory if need be.
	pub fn write(&self, path: &Path) -> std::io::Result<()> {
		let dir = path
			.parent()
			.filter(|p| !p.as_os_str().is_empty())
			.unwrap_or(Path::new("."));
		std::fs::create_dir_all(dir)?;
		let mut file = tempfile::NamedTempFile::new_in(dir)?;
		file.write_all(self.to_json().as_bytes())?;
		file.persist(path).map_err(|e| e.error)?;
		Ok(())
	}

	/// The first template whose inputs differ between two freezes, or `None`. The build
	/// that wrote each is not an input.
	pub fn first_difference(&self, other: &Frozen) -> Option<String> {
		let specs: BTreeSet<&String> = self.specs.keys().chain(other.specs.keys()).collect();
		for spec in specs {
			let (Some(a), Some(b)) = (self.specs.get(spec), other.specs.get(spec)) else {
				return Some(format!("spec '{spec}' differs"));
			};
			let cases: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
			if let Some(case) = cases.into_iter().find(|c| a.get(*c) != b.get(*c)) {
				return Some(format!("case '{case}' in '{spec}' differs"));
			}
		}
		None
	}
}

/// Where the frozen inputs of `output` go: `results.json` → `results.cases.json`.
pub fn beside(output: &Path) -> PathBuf {
	let stem = output.file_stem().unwrap_or_default().to_string_lossy();
	output.with_file_name(format!("{stem}.cases.json"))
}

/// Whether a fresh run may write `new` over what `path` holds: only if it holds nothing,
/// or the same inputs, or `force` says to replace it.
pub fn check_replaceable(path: &Path, new: &Frozen, force: bool) -> Result<(), String> {
	if force || !path.exists() {
		return Ok(());
	}
	let shown = path.display();
	match Frozen::load(path) {
		Ok(old) => match old.first_difference(new) {
			None => Ok(()),
			Some(difference) => Err(format!(
				"{shown} holds other inputs ({difference}): grade with --replay {shown} to use them again, or --fresh to replace them"
			)),
		},
		Err(e) => Err(format!("{e}: grade with --fresh to replace it")),
	}
}

/// A frozen template, checked against the spec's settings and against itself, and taken
/// as it is — its generator version and seed included.
pub fn replay_entry(
	case: &str,
	inputs: &Inputs,
	entry: &Generated,
) -> Result<Generated, Vec<String>> {
	const RESTORE: &str = "restore it, or draw new inputs instead of replaying these";
	let frozen = &entry.inputs;
	let mut problems = Vec::new();
	if inputs.args != frozen.args {
		problems.push(format!(
			"the spec's args are {}; the frozen inputs used {}: {RESTORE}",
			shown_args(inputs),
			shown_args(frozen)
		));
	}
	if inputs.samples != frozen.samples {
		problems.push(format!(
			"the spec's samples differ from the frozen inputs' {}: {RESTORE}",
			Value::from(frozen.samples.clone())
		));
	}
	let count = |i: &Inputs| i.random.as_ref().map_or(0, |r| r.count);
	if count(inputs) != count(frozen) {
		problems.push(format!(
			"the spec says count = {}; the frozen inputs used count = {}: {RESTORE}",
			count(inputs),
			count(frozen)
		));
	}
	// A seed matters only where something was drawn; "random" accepts whichever was.
	if let Some(recorded) = entry.seed {
		let (wanted, by_default) = match inputs.random.as_ref().and_then(|r| r.seed) {
			Some(Seed::Random) => (None, false),
			Some(Seed::Fixed(n)) => (Some(n), false),
			None => (Some(0), true),
		};
		if let Some(n) = wanted.filter(|n| *n != recorded) {
			problems.push(format!(
				"the spec says seed = {n}{}; the frozen inputs were drawn with seed {recorded}: {RESTORE}",
				if by_default { " (by default)" } else { "" }
			));
		}
	}
	problems.extend(consistency(case, entry));
	if problems.is_empty() {
		Ok(entry.clone())
	} else {
		Err(problems)
	}
}

fn shown_args(inputs: &Inputs) -> String {
	let args: Vec<String> = inputs
		.args
		.iter()
		.map(|p| format!("{} = {:?}", p.name, p.rule))
		.collect();
	format!("[{}]", args.join(", "))
}

/// What an entry says of itself must hold: its rows are exactly its samples and draws, in
/// order and by name, each a full row of plain values.
fn consistency(case: &str, entry: &Generated) -> Vec<String> {
	let inputs = &entry.inputs;
	let arity = inputs.args.len();
	let draws = inputs.random.as_ref().map_or(0, |r| r.count);
	let mut problems = Vec::new();
	if draws > MAX_COUNT {
		return vec![format!(
			"the frozen inputs draw {draws} cases, past {MAX_COUNT}"
		)];
	}
	let rows: Vec<(&str, Origin)> = entry
		.cases
		.iter()
		.map(|c| (c.name.as_str(), c.origin))
		.collect();
	let expected: Vec<(String, Origin)> = origins(inputs)
		.map(|o| (concrete_name(case, o), o))
		.collect();
	if rows
		!= expected
			.iter()
			.map(|(n, o)| (n.as_str(), *o))
			.collect::<Vec<_>>()
	{
		problems.push(format!(
			"the frozen rows are not samples 0..{} then draws 0..{draws}, named as ScriptMark names them",
			inputs.samples.len()
		));
	}
	let drew = draws > 0 && arity > 0;
	if entry.seed.is_some() != drew || entry.seed_source.is_some() != drew {
		problems.push("the frozen seed does not fit the frozen settings".into());
	}
	for row in &entry.cases {
		let at = format!("the frozen row '{}'", row.name);
		if let Origin::Sample(j) = row.origin
			&& inputs.samples.get(j) != Some(&row.args)
		{
			problems.push(format!("{at} is not sample {j} as written"));
		}
		if row.args.len() != arity {
			problems.push(format!(
				"{at} has {} value{}, but the template has {arity} parameter{}",
				row.args.len(),
				if row.args.len() == 1 { "" } else { "s" },
				if arity == 1 { "" } else { "s" }
			));
		}
		if row.args.iter().any(holds_null) {
			problems.push(format!("{at} holds null, which no test can pass"));
		}
		if let Some(name) = crate::spec_loader::refs(&row.args).first() {
			problems.push(format!(
				"{at} holds '${name}', a reference: frozen inputs are values, so write '$${name}' for the literal text"
			));
		}
	}
	problems
}

fn holds_null(value: &Value) -> bool {
	match value {
		Value::Null => true,
		Value::Array(items) => items.iter().any(holds_null),
		Value::Object(map) => map.values().any(holds_null),
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{Param, Random, Seed};
	use crate::runner::generation::{SeedSource, generate};
	use crate::runner::generator::Rule;
	use rand_chacha::ChaCha8Rng;
	use rand_chacha::rand_core::SeedableRng;
	use serde_json::{Value, json};

	fn inputs(count: usize, seed: Option<Seed>) -> Inputs {
		Inputs {
			args: vec![
				Param {
					name: "value".into(),
					rule: "int(-100, 100)".into(),
				},
				Param {
					name: "low".into(),
					rule: "int(-49, -26)".into(),
				},
			],
			samples: vec![vec![json!(-30), json!(-30)], vec![json!(30), json!(-30)]],
			random: Some(Random { count, seed }),
		}
	}

	fn made(inputs: &Inputs, seed: (u64, SeedSource)) -> Generated {
		generate("clamp", inputs, Some(seed)).unwrap()
	}

	fn frozen(entry: Generated) -> Frozen {
		Frozen {
			format: FORMAT,
			scriptmark: "test".into(),
			specs: BTreeMap::from([(
				"spec".to_string(),
				BTreeMap::from([("clamp".to_string(), entry)]),
			)]),
		}
	}

	#[test]
	fn test_beside_names_the_file() {
		assert_eq!(
			beside(Path::new("output/results.json")),
			Path::new("output/results.cases.json")
		);
		assert_eq!(beside(Path::new("out/r")), Path::new("out/r.cases.json"));
		assert_eq!(
			beside(Path::new("run.1.json")),
			Path::new("run.1.cases.json")
		);
	}

	#[test]
	fn test_write_creates_the_directory_and_leaves_one_whole_file() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("new/dir/results.cases.json");
		let f = frozen(made(&inputs(3, None), (0, SeedSource::Default)));
		f.write(&path).unwrap();
		f.write(&path).unwrap();
		assert_eq!(Frozen::load(&path).unwrap(), f);
		let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
			.unwrap()
			.map(|e| e.unwrap().file_name())
			.collect();
		assert_eq!(names, ["results.cases.json"]);
	}

	#[test]
	fn test_floats_round_trip_bit_exact() {
		let rule = Rule::parse("float(-1e9, 1e9)").unwrap();
		let mut rng = ChaCha8Rng::seed_from_u64(1);
		let mut floats: Vec<f64> = (0..10_000)
			.map(|_| rule.draw(&mut rng).as_f64().unwrap())
			.collect();
		floats.extend([
			0.1,
			0.30000000000000004,
			1e-300,
			f64::MIN_POSITIVE,
			5e-324,
			f64::MAX,
			-0.0,
		]);
		let mut entry = made(&inputs(1, None), (0, SeedSource::Default));
		entry.cases[2].args = floats.iter().map(|f| json!(f)).collect();
		let back = Frozen::from_json(&frozen(entry).to_json()).unwrap();
		let got: Vec<u64> = back.specs["spec"]["clamp"].cases[2]
			.args
			.iter()
			.map(|v| v.as_f64().unwrap().to_bits())
			.collect();
		assert_eq!(got, floats.iter().map(|f| f.to_bits()).collect::<Vec<_>>());
	}

	#[test]
	fn test_a_newer_format_is_refused_by_its_number() {
		let mut value: Value = serde_json::from_str(
			&frozen(made(&inputs(1, None), (0, SeedSource::Default))).to_json(),
		)
		.unwrap();
		value["format"] = json!(2);
		value["specs"]["spec"]["clamp"]["cases"][0]["answer"] = json!(5);
		let e = Frozen::from_json(&value.to_string()).unwrap_err();
		assert!(
			e.contains("format 2") && e.contains("reads format 1"),
			"{e}"
		);
		let e = Frozen::from_json("{\"specs\": {}}").unwrap_err();
		assert!(e.contains("no format"), "{e}");
		let e = Frozen::from_json("not json").unwrap_err();
		assert!(!e.is_empty());
	}

	#[test]
	fn test_a_fresh_run_replaces_only_the_same_inputs_unless_forced() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("results.cases.json");
		let old = frozen(made(&inputs(3, None), (0, SeedSource::Default)));
		let new = frozen(made(
			&inputs(3, Some(Seed::Random)),
			(81234, SeedSource::Drawn),
		));

		assert_eq!(
			check_replaceable(&path, &new, false),
			Ok(()),
			"nothing there yet"
		);
		old.write(&path).unwrap();
		assert_eq!(
			check_replaceable(&path, &old, false),
			Ok(()),
			"the same inputs"
		);
		let e = check_replaceable(&path, &new, false).unwrap_err();
		assert!(
			e.contains("holds other inputs (case 'clamp' in 'spec' differs)"),
			"{e}"
		);
		assert!(e.contains("--replay") && e.contains("--fresh"), "{e}");
		assert_eq!(check_replaceable(&path, &new, true), Ok(()), "--fresh");

		std::fs::write(&path, "garbage").unwrap();
		let e = check_replaceable(&path, &new, false).unwrap_err();
		assert!(e.contains("--fresh"), "{e}");
	}

	#[test]
	fn test_the_first_difference_names_the_template() {
		let a = frozen(made(&inputs(3, None), (0, SeedSource::Default)));
		assert_eq!(a.first_difference(&a), None);
		let b = frozen(made(&inputs(4, None), (0, SeedSource::Default)));
		assert_eq!(
			a.first_difference(&b).as_deref(),
			Some("case 'clamp' in 'spec' differs")
		);
		let mut c = a.clone();
		c.specs.get_mut("spec").unwrap().insert(
			"other".into(),
			made(&inputs(1, None), (0, SeedSource::Default)),
		);
		assert_eq!(
			a.first_difference(&c).as_deref(),
			Some("case 'other' in 'spec' differs")
		);
		let mut d = a.clone();
		d.specs.insert("gone".into(), BTreeMap::new());
		assert_eq!(
			a.first_difference(&d).as_deref(),
			Some("spec 'gone' differs")
		);
		let mut e = a.clone();
		e.scriptmark = "another build".into();
		assert_eq!(a.first_difference(&e), None, "the writer is not an input");
	}

	#[test]
	fn test_replay_takes_the_entry_as_it_is() {
		let spec = inputs(3, Some(Seed::Random));
		let mut entry = made(&spec, (81234, SeedSource::Drawn));
		entry.generator = 999;
		entry.cases[3].args = vec![json!(5000), json!(5000)];
		assert_eq!(replay_entry("clamp", &spec, &entry), Ok(entry.clone()));
		// Pasting the drawn seed back into the spec still replays it.
		assert_eq!(
			replay_entry("clamp", &inputs(3, Some(Seed::Fixed(81234))), &entry),
			Ok(entry.clone())
		);
	}

	#[test]
	fn test_replay_refuses_what_it_cannot_honour() {
		let spec = inputs(3, None);
		let entry = made(&spec, (0, SeedSource::Default));
		let refused = |spec: &Inputs, entry: &Generated, needle: &str| {
			let problems = replay_entry("clamp", spec, entry).unwrap_err();
			assert!(
				problems.iter().any(|p| p.contains(needle)),
				"expected {needle:?} in {problems:?}"
			);
		};

		refused(
			&inputs(30, None),
			&entry,
			"the spec says count = 30; the frozen inputs used count = 3",
		);
		let mut rule = spec.clone();
		rule.args[0].rule = "int(-99, 100)".into();
		refused(&rule, &entry, "args");
		let mut order = spec.clone();
		order.args.reverse();
		refused(&order, &entry, "args");
		let mut samples = spec.clone();
		samples.samples[1][0] = json!(31);
		refused(&samples, &entry, "samples");
		refused(
			&inputs(3, Some(Seed::Fixed(5))),
			&entry,
			"the spec says seed = 5; the frozen inputs were drawn with seed 0",
		);

		let mut truncated = entry.clone();
		truncated.cases.pop();
		refused(&spec, &truncated, "rows");
		let mut renamed = entry.clone();
		renamed.cases[3].name = "clamp [7]".into();
		refused(&spec, &renamed, "rows");
		let mut edited = entry.clone();
		edited.cases[0].args[0] = json!(0);
		refused(&spec, &edited, "sample 0");
		let mut short = entry.clone();
		short.cases[4].args.pop();
		refused(&spec, &short, "1 value, but the template has 2 parameters");
		let mut null = entry.clone();
		null.cases[4].args[1] = Value::Null;
		refused(&spec, &null, "null");
		let mut reference = entry.clone();
		reference.cases[4].args[1] = json!(["$LIMIT"]);
		refused(&spec, &reference, "'$LIMIT', a reference");
	}
}
