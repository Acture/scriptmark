//! What a unit's student side may hold beyond what it takes idle, when the spec does not
//! say in MiB: estimated from the space the teacher allows the algorithm (`space`, O(n²)
//! unless declared) and the size of the unit's test data.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::models::{TestCase, TestSpec};
use crate::runner::executor::UnitPlan;

/// How much memory an algorithm may use, as the size of its data grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
	Constant,
	Logarithmic,
	Linear,
	Linearithmic,
	Quadratic,
}

/// What a unit may hold where nothing is declared: lenient, as an exercise's data is small.
pub const DEFAULT_SPACE: Space = Space::Quadratic;

/// The least an estimate gives, in MiB: what a small exercise holds, above the noise of
/// loading a module.
pub const ESTIMATE_FLOOR_MB: u64 = 16;

/// The most an estimate gives, in MiB, and what a unit whose data cannot be sized before
/// grading gets: past it, a teacher declares `memory_mb`.
pub const ESTIMATE_CAP_MB: u64 = 1024;

/// Bytes per step of the growth: about 64 bytes a Python value takes, eight times over
/// for the input held, copies of it, the result and ScriptMark's encodings of it.
const BYTES_PER_STEP: f64 = 512.0;

impl Space {
	/// `O(1)`, `O(log n)`, `O(n)`, `O(n log n)` or `O(n^2)` (also `O(n²)`, `O(n**2)`).
	pub fn parse(text: &str) -> Option<Self> {
		let squeezed: String = text
			.chars()
			.filter(|c| !c.is_whitespace())
			.collect::<String>()
			.to_lowercase();
		let inner = squeezed.strip_prefix("o(")?.strip_suffix(')')?;
		Some(match inner {
			"1" => Space::Constant,
			"logn" => Space::Logarithmic,
			"n" => Space::Linear,
			"nlogn" => Space::Linearithmic,
			"n^2" | "n²" | "n**2" | "n*n" => Space::Quadratic,
			_ => return None,
		})
	}

	fn grow(self, n: f64) -> f64 {
		let log = n.max(2.0).log2();
		match self {
			Space::Constant => 1.0,
			Space::Logarithmic => log,
			Space::Linear => n,
			Space::Linearithmic => n * log,
			Space::Quadratic => n * n,
		}
	}
}

impl fmt::Display for Space {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Space::Constant => "O(1)",
			Space::Logarithmic => "O(log n)",
			Space::Linear => "O(n)",
			Space::Linearithmic => "O(n log n)",
			Space::Quadratic => "O(n^2)",
		})
	}
}

/// The MiB `space` allows over data of `size` elements, and how it was reached. Data that
/// cannot be sized before grading gets the cap.
pub fn estimate(space: Space, size: Option<u64>) -> (u64, String) {
	let Some(n) = size else {
		return (
			ESTIMATE_CAP_MB,
			format!("{space}, the data's size unknown before grading"),
		);
	};
	let mb = (BYTES_PER_STEP * space.grow(n as f64) / f64::from(1 << 20)).ceil();
	let mb = mb.clamp(ESTIMATE_FLOOR_MB as f64, ESTIMATE_CAP_MB as f64) as u64;
	let elements = if n == 1 { "element" } else { "elements" };
	(mb, format!("{space} over {n} {elements}"))
}

/// What a unit may hold, in MiB, and how an undeclared limit was estimated: the narrowest
/// declaration first — `memory_mb` as written, or `space` estimated over `size` — and
/// `DEFAULT_SPACE` where none is.
pub fn resolve(
	declared: [(Option<u64>, Option<&str>); 2],
	size: Option<u64>,
) -> (u64, Option<String>) {
	let space = declared
		.into_iter()
		.find_map(
			|(memory_mb, space)| match (memory_mb, space.and_then(Space::parse)) {
				(Some(mb), _) => Some(Err(mb)),
				(None, Some(space)) => Some(Ok(space)),
				(None, None) => None,
			},
		)
		.unwrap_or(Ok(DEFAULT_SPACE));
	match space {
		Err(mb) => (mb, None),
		Ok(space) => {
			let (mb, basis) = estimate(space, size);
			(mb, Some(basis))
		}
	}
}

/// The size of a unit's test data, in elements: its calls' arguments and stdin, what its
/// `expected` cases expect, and the bytes of the data files staged for it. None where an
/// argument holds a setup's result, which is not known before grading.
pub fn unit_size<'a>(
	spec: &TestSpec,
	plan: &UnitPlan,
	expected: impl IntoIterator<Item = &'a TestCase>,
) -> Option<u64> {
	let text = |s: &Option<String>| s.as_ref().map_or(0, |s| s.chars().count() as u64);
	let mut total = spec
		.meta
		.data_files
		.iter()
		.map(|rel| bytes_under(&spec.dir.join(rel)))
		.fold(0u64, u64::saturating_add);
	total = total.saturating_add(plan.script.as_ref().map_or(0, |s| text(&s.stdin)));
	for call in plan.setup.iter().chain(&plan.steps) {
		total = total
			.saturating_add(args_size(&call.args, &spec.vars)?)
			.saturating_add(text(&call.stdin));
	}
	for case in expected {
		if let Some(expect) = &case.expect {
			total = total.saturating_add(value_size(expect, &spec.vars)?);
		}
		total = total.saturating_add(text(&case.expected_stdout));
	}
	Some(total)
}

fn bytes_under(path: &Path) -> u64 {
	match std::fs::metadata(path) {
		Ok(meta) if meta.is_dir() => std::fs::read_dir(path).map_or(0, |entries| {
			entries
				.filter_map(Result::ok)
				.map(|entry| bytes_under(&entry.path()))
				.fold(0, u64::saturating_add)
		}),
		Ok(meta) => meta.len(),
		Err(_) => 0,
	}
}

/// The size of a call's arguments, in elements: a container counts its elements, a string
/// its characters, and an integer argument its value — `fib(30)` and `primes(10**6)` are
/// sized by their number. `$name` reads the var `name`; a reference to a setup result
/// cannot be sized before grading: None.
pub fn args_size(args: &[Value], vars: &BTreeMap<String, Value>) -> Option<u64> {
	args.iter().try_fold(0u64, |total, arg| {
		Some(total.saturating_add(size(arg, true, vars)?))
	})
}

/// The size of an expected value: as an argument's, but an integer is one element.
pub fn value_size(value: &Value, vars: &BTreeMap<String, Value>) -> Option<u64> {
	size(value, false, vars)
}

fn size(value: &Value, top: bool, vars: &BTreeMap<String, Value>) -> Option<u64> {
	Some(match value {
		Value::String(s) if s.starts_with('$') && !s.starts_with("$$") => {
			return size(vars.get(&s[1..])?, top, vars);
		}
		Value::String(s) => s.chars().count() as u64,
		Value::Number(number) if top => number
			.as_i64()
			.map(i64::unsigned_abs)
			.or_else(|| number.as_u64())
			.unwrap_or(1),
		Value::Array(items) => items.iter().try_fold(0u64, |total, item| {
			Some(total.saturating_add(size(item, false, vars)?.max(1)))
		})?,
		Value::Object(map) => map.values().try_fold(0u64, |total, item| {
			Some(total.saturating_add(size(item, false, vars)?.saturating_add(1)))
		})?,
		_ => 1,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn test_space_is_written_as_big_o() {
		for (text, space) in [
			("O(1)", Space::Constant),
			("O(log n)", Space::Logarithmic),
			("o(n)", Space::Linear),
			("O(n log n)", Space::Linearithmic),
			("O(n^2)", Space::Quadratic),
			("O(n²)", Space::Quadratic),
			("O( n**2 )", Space::Quadratic),
		] {
			assert_eq!(Space::parse(text), Some(space), "{text}");
		}
		for text in ["n", "O(n^3)", "O(2^n)", "O()", "linear"] {
			assert_eq!(Space::parse(text), None, "{text}");
		}
	}

	#[test]
	fn test_an_estimate_grows_with_the_data_between_floor_and_cap() {
		assert_eq!(estimate(Space::Quadratic, Some(10)).0, ESTIMATE_FLOOR_MB);
		assert_eq!(
			estimate(Space::Quadratic, Some(1000)),
			(489, "O(n^2) over 1000 elements".to_string())
		);
		assert_eq!(estimate(Space::Linear, Some(1_000_000)).0, 489);
		assert_eq!(estimate(Space::Quadratic, Some(100_000)).0, ESTIMATE_CAP_MB);
		assert_eq!(
			estimate(Space::Constant, Some(u64::MAX)).0,
			ESTIMATE_FLOOR_MB
		);
		assert_eq!(estimate(Space::Linear, None).0, ESTIMATE_CAP_MB);
		assert_eq!(estimate(Space::Linear, Some(1)).1, "O(n) over 1 element");
	}

	#[test]
	fn test_the_narrowest_declaration_applies() {
		let n = Some(1000);
		assert_eq!(
			resolve([(None, None), (None, None)], n),
			(489, Some("O(n^2) over 1000 elements".into()))
		);
		assert_eq!(
			resolve([(Some(32), None), (None, Some("O(1)"))], n),
			(32, None)
		);
		assert_eq!(
			resolve([(None, Some("O(1)")), (Some(512), None)], n).0,
			ESTIMATE_FLOOR_MB
		);
		assert_eq!(resolve([(None, None), (Some(512), None)], n), (512, None));
		assert_eq!(
			resolve([(None, None), (None, Some("O(n)"))], None).0,
			ESTIMATE_CAP_MB
		);
	}

	#[test]
	fn test_data_is_sized_by_elements_characters_and_integer_arguments() {
		let vars = BTreeMap::from([
			("N".to_string(), json!(30)),
			("XS".to_string(), json!([1, 2])),
		]);
		assert_eq!(args_size(&[json!([3, 1, 5, 2])], &vars), Some(4));
		assert_eq!(args_size(&[json!(3), json!(5)], &vars), Some(8));
		assert_eq!(args_size(&[json!(1_000_000)], &vars), Some(1_000_000));
		assert_eq!(args_size(&[json!(-7), json!(2.5)], &vars), Some(8));
		assert_eq!(args_size(&[json!("hello")], &vars), Some(5));
		assert_eq!(
			args_size(&[json!({"a": [1, 2], "b": "xy"})], &vars),
			Some(6)
		);
		assert_eq!(args_size(&[json!("$N"), json!("$XS")], &vars), Some(32));
		assert_eq!(args_size(&[json!("$$literal")], &vars), Some(9));
		assert_eq!(args_size(&[json!("$data")], &vars), None);
		assert_eq!(value_size(&json!(8_000_000), &vars), Some(1));
		assert_eq!(value_size(&json!([[1, 2], []]), &vars), Some(3));
	}
}
