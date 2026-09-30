//! Generation rules: what a teacher writes for one parameter, parsed once, then drawn.
//!
//! A rule that parses cannot fail to draw. Values come from our own mapping on a portable
//! ChaCha8 stream, so the same seed gives the same values on every platform and with
//! every `rand` release; `GENERATOR_VERSION` names that mapping.

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::RngCore;
use serde_json::Value;

use crate::spec_loader::{contains_null, refs};

/// The version of the value mapping. Any change to what a seed draws bumps it.
pub const GENERATOR_VERSION: u32 = 1;

/// How deeply `list` may nest.
pub const MAX_LIST_DEPTH: usize = 8;

/// A parsed generation rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
	/// `int(min, max)` — an integer in `[min, max]`.
	Int { min: i64, max: i64 },
	/// `float(min, max)` — a float in `[min, max]`.
	Float { min: f64, max: f64 },
	/// `bool()`.
	Bool,
	/// `str(min_len, max_len)` — lowercase letters and digits.
	Str { min: usize, max: usize },
	/// `choice([v1, v2, ...])` — one of the listed JSON values.
	Choice(Vec<Value>),
	/// `list(rule, min_len, max_len)`.
	List {
		item: Box<Rule>,
		min: usize,
		max: usize,
	},
}

/// Why a rule does not parse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct RuleError(pub String);

const RULES: &str = "int, float, bool, str, choice, list";
const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn fail<T>(message: String) -> Result<T, RuleError> {
	Err(RuleError(message))
}

impl Rule {
	pub fn parse(expr: &str) -> Result<Rule, RuleError> {
		parse_at(expr, 0)
	}

	/// Draw one value. Cannot fail: everything that could is refused by `parse`.
	pub fn draw(&self, rng: &mut ChaCha8Rng) -> Value {
		match self {
			Rule::Int { min, max } => Value::from(int_in(rng, *min, *max)),
			Rule::Float { min, max } => Value::from(float_in(rng, *min, *max)),
			Rule::Bool => Value::from(rng.next_u64() >> 63 == 1),
			Rule::Str { min, max } => {
				let len = len_in(rng, *min, *max);
				let text: String = (0..len)
					.map(|_| ALPHABET[below(rng, ALPHABET.len() as u128) as usize] as char)
					.collect();
				Value::from(text)
			}
			Rule::Choice(items) => items[below(rng, items.len() as u128) as usize].clone(),
			Rule::List { item, min, max } => {
				let len = len_in(rng, *min, *max);
				Value::Array((0..len).map(|_| item.draw(rng)).collect())
			}
		}
	}

	/// The most values or characters one draw can produce, saturating.
	pub fn max_size(&self) -> u64 {
		match self {
			Rule::Int { .. } | Rule::Float { .. } | Rule::Bool => 1,
			Rule::Choice(items) => items.iter().map(value_size).max().unwrap_or(1),
			Rule::Str { max, .. } => (*max as u64).max(1),
			Rule::List { item, max, .. } => (*max as u64)
				.saturating_mul(item.max_size())
				.saturating_add(1),
		}
	}
}

fn parse_at(expr: &str, depth: usize) -> Result<Rule, RuleError> {
	let expr = expr.trim();
	let Some((name, inner)) = expr
		.find('(')
		.filter(|_| expr.ends_with(')'))
		.map(|open| (expr[..open].trim(), &expr[open + 1..expr.len() - 1]))
	else {
		return fail(format!(
			"unknown rule '{expr}': the rules are {RULES}, written like int(0, 10)"
		));
	};
	match name {
		"int" => {
			let [a, b] = arguments(name, inner)?;
			let (min, max) = (integer(a)?, integer(b)?);
			ordered(expr, min, max)?;
			Ok(Rule::Int { min, max })
		}
		"float" => {
			let [a, b] = arguments(name, inner)?;
			let (min, max) = (float(a)?, float(b)?);
			ordered(expr, min, max)?;
			if !(max - min).is_finite() {
				return fail(format!("the span of {expr} is not a finite number"));
			}
			Ok(Rule::Float { min, max })
		}
		"bool" if inner.trim().is_empty() => Ok(Rule::Bool),
		"bool" => fail(format!("bool() takes no arguments, got {expr}")),
		"str" => {
			let [a, b] = arguments(name, inner)?;
			let (min, max) = (length(a)?, length(b)?);
			ordered(expr, min, max)?;
			Ok(Rule::Str { min, max })
		}
		"choice" => choice(inner).map(Rule::Choice),
		"list" => {
			if depth >= MAX_LIST_DEPTH {
				return fail(format!(
					"list is nested deeper than {MAX_LIST_DEPTH} levels"
				));
			}
			let [item, a, b] = arguments(name, inner)?;
			let item = parse_at(item, depth + 1)?;
			let (min, max) = (length(a)?, length(b)?);
			ordered(expr, min, max)?;
			Ok(Rule::List {
				item: Box::new(item),
				min,
				max,
			})
		}
		_ => fail(format!("unknown rule '{name}': the rules are {RULES}")),
	}
}

/// Exactly `N` comma-separated arguments, split outside brackets and JSON strings.
fn arguments<'a, const N: usize>(name: &str, inner: &'a str) -> Result<[&'a str; N], RuleError> {
	let parts = split_top(inner);
	parts.try_into().or_else(|parts: Vec<&str>| {
		fail(format!(
			"{name}(...) takes {N} arguments, got {}",
			parts.len()
		))
	})
}

fn split_top(inner: &str) -> Vec<&str> {
	if inner.trim().is_empty() {
		return Vec::new();
	}
	let (mut parts, mut start, mut depth) = (Vec::new(), 0, 0i32);
	let (mut in_string, mut escaped) = (false, false);
	for (i, c) in inner.char_indices() {
		if in_string {
			match c {
				_ if escaped => escaped = false,
				'\\' => escaped = true,
				'"' => in_string = false,
				_ => {}
			}
			continue;
		}
		match c {
			'"' => in_string = true,
			'(' | '[' | '{' => depth += 1,
			')' | ']' | '}' => depth -= 1,
			',' if depth == 0 => {
				parts.push(&inner[start..i]);
				start = i + 1;
			}
			_ => {}
		}
	}
	parts.push(&inner[start..]);
	parts
}

fn integer(text: &str) -> Result<i64, RuleError> {
	let text = text.trim();
	text.parse().or_else(|_| {
		fail(format!(
			"'{text}' is not an integer between {} and {}",
			i64::MIN,
			i64::MAX
		))
	})
}

fn float(text: &str) -> Result<f64, RuleError> {
	let text = text.trim();
	match text.parse::<f64>() {
		Ok(value) if value.is_finite() => Ok(value),
		_ => fail(format!("'{text}' is not a finite number")),
	}
}

fn length(text: &str) -> Result<usize, RuleError> {
	let text = text.trim();
	text.parse().or_else(|_| {
		fail(format!(
			"'{text}' is not a length (a whole number, 0 or more)"
		))
	})
}

fn ordered<T: PartialOrd + std::fmt::Display>(expr: &str, min: T, max: T) -> Result<(), RuleError> {
	if min > max {
		return fail(format!(
			"{expr}: the minimum {min} is greater than the maximum {max}"
		));
	}
	Ok(())
}

fn choice(inner: &str) -> Result<Vec<Value>, RuleError> {
	const SPELLING: &str =
		r#"choice needs a JSON array, e.g. choice([1, 2, 3]) or choice(["a", "b"])"#;
	let items = match serde_json::from_str::<Value>(inner.trim()) {
		Ok(Value::Array(items)) => items,
		Ok(_) => return fail(SPELLING.to_string()),
		Err(e) => return fail(format!("{SPELLING}: {e}")),
	};
	if items.is_empty() {
		return fail("choice([]) is empty: list at least one value".into());
	}
	if items.iter().any(contains_null) {
		return fail("choice holds null, which no test can pass as an argument".into());
	}
	if let Some(name) = refs(&items).first() {
		return fail(format!(
			"choice holds '${name}', a reference: a generated value is frozen as written, so write '$${name}' for the literal text"
		));
	}
	// serde_json reads an integer past 64 bits as a float; the text says what was meant.
	if let Some(n) = integer_literals(inner)
		.into_iter()
		.find(|n| n.parse::<i64>().is_err())
	{
		return fail(format!("choice holds {n}, beyond a 64-bit signed integer"));
	}
	Ok(items)
}

/// The integer literals written in JSON `text`, outside its strings.
fn integer_literals(text: &str) -> Vec<&str> {
	let bytes = text.as_bytes();
	let (mut literals, mut i) = (Vec::new(), 0);
	let (mut in_string, mut escaped) = (false, false);
	while i < bytes.len() {
		let b = bytes[i];
		if in_string {
			match b {
				_ if escaped => escaped = false,
				b'\\' => escaped = true,
				b'"' => in_string = false,
				_ => {}
			}
		} else if b == b'"' {
			in_string = true;
		} else if b == b'-' || b.is_ascii_digit() {
			let start = i;
			while i < bytes.len()
				&& matches!(bytes[i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
			{
				i += 1;
			}
			let literal = &text[start..i];
			if !literal.contains(['.', 'e', 'E']) {
				literals.push(literal);
			}
			continue;
		}
		i += 1;
	}
	literals
}

/// How many values or characters a JSON value holds, counting itself.
fn value_size(value: &Value) -> u64 {
	match value {
		Value::String(text) => (text.chars().count() as u64).max(1),
		Value::Array(items) => items.iter().map(value_size).fold(1, u64::saturating_add),
		Value::Object(map) => map.values().map(value_size).fold(1, u64::saturating_add),
		_ => 1,
	}
}

/// A uniform integer in `0..span`, for `1 <= span <= 2^64`, by rejection: no modulo bias.
fn below(rng: &mut ChaCha8Rng, span: u128) -> u64 {
	if span > u64::MAX as u128 {
		return rng.next_u64();
	}
	let span = span as u64;
	let rejected = (u64::MAX % span + 1) % span;
	loop {
		let x = rng.next_u64();
		if x <= u64::MAX - rejected {
			return x % span;
		}
	}
}

fn int_in(rng: &mut ChaCha8Rng, min: i64, max: i64) -> i64 {
	let span = (max as i128 - min as i128 + 1) as u128;
	(min as i128 + below(rng, span) as i128) as i64
}

fn len_in(rng: &mut ChaCha8Rng, min: usize, max: usize) -> usize {
	min + below(rng, (max - min) as u128 + 1) as usize
}

/// `min + (max - min)·u` for a 53-bit `u` in `[0, 1)`, held to `[min, max]` against rounding.
fn float_in(rng: &mut ChaCha8Rng, min: f64, max: f64) -> f64 {
	let unit = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
	(min + (max - min) * unit).clamp(min, max)
}

#[cfg(test)]
mod tests {
	use super::*;
	use rand_chacha::rand_core::SeedableRng;
	use serde_json::json;

	fn rng(seed: u64) -> ChaCha8Rng {
		ChaCha8Rng::seed_from_u64(seed)
	}

	fn parse(expr: &str) -> Rule {
		Rule::parse(expr).unwrap_or_else(|e| panic!("{expr}: {e}"))
	}

	#[test]
	fn test_rules_parse_to_their_ast() {
		assert_eq!(
			parse("int(-100, 100)"),
			Rule::Int {
				min: -100,
				max: 100
			}
		);
		assert_eq!(parse(" float(0, 1.5) "), Rule::Float { min: 0.0, max: 1.5 });
		assert_eq!(parse("bool()"), Rule::Bool);
		assert_eq!(parse("str(3, 10)"), Rule::Str { min: 3, max: 10 });
		assert_eq!(
			parse(r#"choice([1, "a", [2, 3]])"#),
			Rule::Choice(vec![json!(1), json!("a"), json!([2, 3])])
		);
		assert_eq!(
			parse("list(int(0, 10), 3, 5)"),
			Rule::List {
				item: Box::new(Rule::Int { min: 0, max: 10 }),
				min: 3,
				max: 5
			}
		);
		// A comma or bracket inside a JSON string does not split the arguments.
		assert_eq!(
			parse(r#"list(choice(["a)", "b,c"]), 1, 3)"#),
			Rule::List {
				item: Box::new(Rule::Choice(vec![json!("a)"), json!("b,c")])),
				min: 1,
				max: 3
			}
		);
		assert_eq!(
			parse(r#"choice(["$$5"])"#),
			Rule::Choice(vec![json!("$$5")])
		);
		// A float may be as large as it likes; only an integer must fit 64 bits.
		assert_eq!(
			parse(r#"choice([1e20, -9223372036854775808, "99999999999999999999"])"#),
			Rule::Choice(vec![
				json!(1e20),
				json!(i64::MIN),
				json!("99999999999999999999")
			])
		);
	}

	#[test]
	fn test_bad_rules_are_refused_with_the_reason() {
		let cases = [
			("rand(1, 2)", "unknown rule"),
			("int(1, 2", "unknown rule"),
			("int(1)", "takes 2 arguments"),
			("int(1, 2, 3)", "takes 2 arguments"),
			("int(a, 2)", "not an integer"),
			("int(5, 1)", "greater than"),
			("float(1.0, 0.5)", "greater than"),
			("float(nan, 1)", "finite"),
			("float(0, inf)", "finite"),
			("float(-1e308, 1e308)", "finite"),
			("str(-1, 3)", "not a length"),
			("str(4, 2)", "greater than"),
			("bool(1)", "takes no arguments"),
			("choice(1, 2)", "JSON array"),
			("choice({\"a\": 1})", "JSON array"),
			("choice([])", "empty"),
			("choice([1, null])", "null"),
			(r#"choice(["$x"])"#, "reference"),
			(r#"choice([[{"k": "$y"}]])"#, "reference"),
			("choice([18446744073709551615])", "64-bit"),
			("choice([18446744073709551616])", "64-bit"),
			("choice([-9223372036854775809])", "64-bit"),
			("choice([[1, 99999999999999999999]])", "64-bit"),
			("list(nope(), 0, 2)", "unknown rule"),
			("list(int(0, 1), 3)", "takes 3 arguments"),
			("list(int(0, 1), 3, 1)", "greater than"),
			(
				"list(list(list(list(list(list(list(list(list(bool(), 1, 1), 1, 1), 1, 1), 1, 1), 1, 1), 1, 1), 1, 1), 1, 1), 1, 1)",
				"nested deeper",
			),
		];
		for (expr, needle) in cases {
			match Rule::parse(expr) {
				Ok(rule) => panic!("{expr} parsed as {rule:?}"),
				Err(e) => assert!(e.0.contains(needle), "{expr}: '{e}' lacks '{needle}'"),
			}
		}
	}

	#[test]
	fn test_draws_stay_within_their_bounds() {
		let mut r = rng(7);
		for _ in 0..2000 {
			let n = parse("int(-3, 4)").draw(&mut r).as_i64().unwrap();
			assert!((-3..=4).contains(&n));
			let f = parse("float(-0.5, 0.25)").draw(&mut r).as_f64().unwrap();
			assert!((-0.5..=0.25).contains(&f));
			let s = parse("str(2, 4)").draw(&mut r);
			let s = s.as_str().unwrap();
			assert!((2..=4).contains(&s.len()));
			assert!(
				s.bytes()
					.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
			);
			let l = parse("list(int(0, 1), 0, 2)").draw(&mut r);
			assert!(l.as_array().unwrap().len() <= 2);
		}
	}

	#[test]
	fn test_every_value_of_a_small_span_is_drawn() {
		let mut r = rng(1);
		let mut seen = std::collections::BTreeSet::new();
		for _ in 0..500 {
			seen.insert(parse("int(-2, 2)").draw(&mut r).as_i64().unwrap());
			seen.insert(parse(r#"choice([10, 20])"#).draw(&mut r).as_i64().unwrap());
		}
		assert_eq!(
			seen.into_iter().collect::<Vec<_>>(),
			[-2, -1, 0, 1, 2, 10, 20]
		);
	}

	#[test]
	fn test_int_takes_the_full_range_and_a_single_value() {
		let mut r = rng(3);
		let full = parse(&format!("int({}, {})", i64::MIN, i64::MAX));
		let drawn: Vec<i64> = (0..64)
			.map(|_| full.draw(&mut r).as_i64().unwrap())
			.collect();
		assert!(drawn.iter().any(|n| *n < 0) && drawn.iter().any(|n| *n > 0));
		assert_eq!(parse("int(9, 9)").draw(&mut r), json!(9));
		assert_eq!(parse("float(2.5, 2.5)").draw(&mut r), json!(2.5));
	}

	#[test]
	fn test_the_same_seed_draws_the_same_values() {
		let rule = parse("list(float(-1, 1), 0, 6)");
		let (mut a, mut b) = (rng(42), rng(42));
		for _ in 0..50 {
			assert_eq!(rule.draw(&mut a), rule.draw(&mut b));
		}
	}

	/// Pins the value mapping. A change here changes what every seed draws: bump
	/// `GENERATOR_VERSION` and update these values together.
	#[test]
	fn test_golden_values_v1() {
		assert_eq!(GENERATOR_VERSION, 1);
		let mut r = rng(42);
		let drawn: Vec<Value> = [
			"int(-1000, 1000)",
			"float(0, 1)",
			"bool()",
			"str(1, 8)",
			r#"choice(["a", 2, [3]])"#,
			"list(int(0, 9), 2, 4)",
		]
		.iter()
		.map(|expr| parse(expr).draw(&mut r))
		.collect();
		assert_eq!(
			drawn,
			[
				json!(929),
				json!(0.950275407672484),
				json!(false),
				json!("w2w"),
				json!("a"),
				json!([4, 0, 3])
			]
		);
	}

	#[test]
	fn test_max_size_bounds_what_one_draw_can_produce() {
		assert_eq!(parse("int(0, 1)").max_size(), 1);
		assert_eq!(parse("str(0, 40)").max_size(), 40);
		assert_eq!(parse("list(str(0, 10), 0, 5)").max_size(), 51);
		// A choice is as large as its largest value.
		assert_eq!(parse(r#"choice(["abc", 1])"#).max_size(), 3);
		assert_eq!(
			parse(r#"choice([[1, [2, 3]], {"k": "abcd"}])"#).max_size(),
			5
		);
		assert_eq!(
			parse(r#"list(choice(["abcdefghij"]), 0, 10)"#).max_size(),
			101
		);
		assert_eq!(
			parse("list(list(bool(), 0, 1000000), 0, 1000000)").max_size(),
			1_000_001_000_001
		);
	}
}
