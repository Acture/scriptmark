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
use crate::spec_loader::{contains_null, plural, refs};

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
	/// creating its directory if need be. It is as readable as any file the umask allows.
	pub fn write(&self, path: &Path) -> std::io::Result<()> {
		let mut file = scratch(path)?;
		file.write_all(self.to_json().as_bytes())?;
		file.persist(path).map_err(|e| e.error)?;
		Ok(())
	}

	/// The first template whose inputs differ between two freezes, or `None`. The inputs
	/// are the rows a student is given: how the seed was spelled or chosen, which generator
	/// drew the rows and which build wrote the file do not make the same rows other inputs.
	pub fn first_difference(&self, other: &Frozen) -> Option<String> {
		let specs: BTreeSet<&String> = self.specs.keys().chain(other.specs.keys()).collect();
		for spec in specs {
			let (Some(a), Some(b)) = (self.specs.get(spec), other.specs.get(spec)) else {
				return Some(format!("spec '{spec}' differs"));
			};
			let cases: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
			let differs = |c: &&String| a.get(*c).map(|g| &g.cases) != b.get(*c).map(|g| &g.cases);
			if let Some(case) = cases.into_iter().find(differs) {
				return Some(format!("case '{case}' in '{spec}' differs"));
			}
		}
		None
	}
}

/// A temporary file beside `path`, its directory made first. Its mode is left to the umask,
/// as `std::fs::write` leaves it, rather than tempfile's owner-only default.
fn scratch(path: &Path) -> std::io::Result<tempfile::NamedTempFile> {
	let dir = path
		.parent()
		.filter(|p| !p.as_os_str().is_empty())
		.unwrap_or(Path::new("."));
	std::fs::create_dir_all(dir)?;
	let mut builder = tempfile::Builder::new();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		builder.permissions(std::fs::Permissions::from_mode(0o666));
	}
	builder.tempfile_in(dir)
}

/// Where the frozen inputs of `output` go: `results.json` → `results.cases.json`.
pub fn beside(output: &Path) -> PathBuf {
	let stem = output.file_stem().unwrap_or_default().to_string_lossy();
	output.with_file_name(format!("{stem}.cases.json"))
}

/// Whether `path` can be written, found out before anything runs: its directory is made,
/// and a scratch file is created in it and removed.
pub fn writable(path: &Path) -> std::io::Result<()> {
	scratch(path).map(drop)
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
	if inputs.draws() != frozen.draws() {
		problems.push(format!(
			"the spec says count = {}; the frozen inputs used count = {}: {RESTORE}",
			inputs.draws(),
			frozen.draws()
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
	let draws = inputs.draws();
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
	let seeded = inputs.seeded();
	if entry.seed.is_some() != seeded || entry.seed_source.is_some() != seeded {
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
				"{at} has {}, but the template has {}",
				plural(row.args.len(), "value"),
				plural(arity, "parameter")
			));
		}
		if row.args.iter().any(contains_null) {
			problems.push(format!("{at} holds null, which no test can pass"));
		}
		if let Some(name) = refs(&row.args).first() {
			problems.push(format!(
				"{at} holds '${name}', a reference: frozen inputs are values, so write '$${name}' for the literal text"
			));
		}
	}
	problems
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

	/// The inputs are the rows a student is given. How the seed was spelled or chosen, and
	/// which generator drew the rows, do not make the same rows other inputs.
	#[test]
	fn test_the_same_rows_are_the_same_inputs_however_the_seed_was_spelled() {
		let drawn = frozen(made(
			&inputs(3, Some(Seed::Random)),
			(81234, SeedSource::Drawn),
		));
		let pasted = frozen(made(
			&inputs(3, Some(Seed::Fixed(81234))),
			(81234, SeedSource::Declared),
		));
		assert_eq!(drawn.first_difference(&pasted), None);
		let omitted = frozen(made(&inputs(3, None), (0, SeedSource::Default)));
		let zero = frozen(made(
			&inputs(3, Some(Seed::Fixed(0))),
			(0, SeedSource::Declared),
		));
		assert_eq!(omitted.first_difference(&zero), None);
		let mut older = zero.clone();
		older
			.specs
			.get_mut("spec")
			.unwrap()
			.get_mut("clamp")
			.unwrap()
			.generator = 999;
		assert_eq!(zero.first_difference(&older), None);
		assert_eq!(
			drawn.first_difference(&zero).as_deref(),
			Some("case 'clamp' in 'spec' differs")
		);
	}

	/// Format 1 as it is written to disk: a rename or a new variant spelling breaks every
	/// file already written, so the layout is pinned here literally.
	#[test]
	fn test_format_1_reads_as_written() {
		let text = r#"{
			"format": 1,
			"scriptmark": "0.3.0",
			"specs": { "clamp": { "clamp": {
				"generator": 1,
				"inputs": {
					"args": [{"value": "int(-100, 100)"}, {"low": "int(-49, -26)"}],
					"samples": [[-30, -30]],
					"random": { "count": 1, "seed": "random" }
				},
				"seed": 81234,
				"seed_source": "drawn",
				"cases": [
					{ "name": "clamp [sample 0]", "origin": {"sample": 0}, "args": [-30, -30] },
					{ "name": "clamp [0]", "origin": {"draw": 0}, "args": [75, -31] }
				]
			} } }
		}"#;
		let f = Frozen::from_json(text).unwrap_or_else(|e| panic!("{e}"));
		let entry = &f.specs["clamp"]["clamp"];
		assert_eq!((entry.generator, entry.seed), (1, Some(81234)));
		assert_eq!(entry.seed_source, Some(SeedSource::Drawn));
		assert_eq!(entry.inputs.names(), ["value", "low"]);
		assert_eq!(
			entry.inputs.random,
			Some(Random {
				count: 1,
				seed: Some(Seed::Random)
			})
		);
		assert_eq!(
			entry.cases[0].origin,
			crate::runner::generation::Origin::Sample(0)
		);
		assert_eq!(
			entry.cases[1].origin,
			crate::runner::generation::Origin::Draw(0)
		);
		assert_eq!(entry.cases[1].args, [json!(75), json!(-31)]);
		let written: Value = serde_json::from_str(&f.to_json()).unwrap();
		assert_eq!(written, serde_json::from_str::<Value>(text).unwrap());
	}

	#[cfg(unix)]
	#[test]
	fn test_the_file_is_as_readable_as_the_results_beside_it() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let results = dir.path().join("results.json");
		std::fs::write(&results, "[]").unwrap();
		let path = beside(&results);
		frozen(made(&inputs(1, None), (0, SeedSource::Default)))
			.write(&path)
			.unwrap();
		let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
		assert_eq!(mode(&path), mode(&results));
	}

	#[test]
	fn test_a_path_that_cannot_be_written_is_found_before_anything_runs() {
		let dir = tempfile::tempdir().unwrap();
		assert!(writable(&dir.path().join("new/dir/cases.json")).is_ok());
		let blocker = dir.path().join("file");
		std::fs::write(&blocker, "").unwrap();
		assert!(writable(&blocker.join("cases.json")).is_err());
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
