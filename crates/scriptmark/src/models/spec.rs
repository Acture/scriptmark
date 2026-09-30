use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Configuration for lint-based code style scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LintConfig {
	/// Lint command template. `{file}` is replaced with student file path.
	pub command: String,
	/// Max warnings that maps to 0% style score.
	#[serde(default = "default_max_warnings")]
	pub max_warnings: usize,
	/// Exit codes that mean the tool ran. Linters commonly exit 1 when they found
	/// something; any other code is the tool failing, not a clean file.
	#[serde(default = "default_ok_exit_codes")]
	pub ok_exit_codes: Vec<i32>,
}

fn default_max_warnings() -> usize {
	10
}

fn default_ok_exit_codes() -> Vec<i32> {
	vec![0, 1]
}

/// The built-in value checkers.
pub const BUILTIN_CHECKERS: &[&str] = &["exact", "approx", "sorted", "set_eq", "contains", "text"];

/// Built-ins that compare against an expectation, and so mean nothing without one.
pub const EXPECT_DEPENDENT_CHECKERS: &[&str] = &["exact", "approx", "set_eq", "contains", "text"];

/// How to check the result of a test case: a built-in's name, or a table naming one kind.
///
/// Deserialised by hand rather than `#[serde(untagged)]`, so that an unknown key such as
/// `regex` is named in the error instead of "did not match any variant".
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum CheckMethod {
	Builtin(String),
	Detailed(CheckSpec),
}

impl<'de> Deserialize<'de> for CheckMethod {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct CheckVisitor;

		impl<'de> Visitor<'de> for CheckVisitor {
			type Value = CheckMethod;

			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a built-in checker name or a table such as { rhai = \"...\" }")
			}

			fn visit_str<E: de::Error>(self, v: &str) -> Result<CheckMethod, E> {
				Ok(CheckMethod::Builtin(v.to_string()))
			}

			fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<CheckMethod, M::Error> {
				CheckSpec::deserialize(de::value::MapAccessDeserializer::new(map))
					.map(CheckMethod::Detailed)
			}
		}

		deserializer.deserialize_any(CheckVisitor)
	}
}

/// A checker written as a table. Exactly one kind may be named.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
	/// Built-in checker name.
	#[serde(default)]
	pub builtin: Option<String>,
	/// Rhai expression over `result`, `expected` and `context`; must evaluate to a bool.
	#[serde(default)]
	pub rhai: Option<String>,
	/// Python script speaking the JSON checker protocol on stdin/stdout.
	#[serde(default)]
	pub python: Option<String>,
	/// Tolerance for the `approx` built-in.
	#[serde(default)]
	pub tolerance: Option<f64>,
	/// A function exported by a teacher module, called in the unit's process with the
	/// live result: `fn(result, expected, **names_in_scope)`.
	#[serde(default)]
	pub function: Option<String>,
}

/// A check, once its spelling has been validated.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
	Builtin {
		name: String,
		tolerance: Option<f64>,
	},
	Rhai(String),
	Python(String),
	Function(String),
}

impl Check {
	/// Whether the check compares against an expectation rather than testing a property.
	pub fn needs_expectation(&self) -> bool {
		matches!(self, Check::Builtin { name, .. } if EXPECT_DEPENDENT_CHECKERS.contains(&name.as_str()))
	}
}

impl CheckMethod {
	/// The one kind this spelling names, or why it names none.
	pub fn resolve(&self) -> Result<Check, String> {
		let builtin = |name: &str, tolerance: Option<f64>| {
			if BUILTIN_CHECKERS.contains(&name) {
				Ok(Check::Builtin {
					name: name.to_string(),
					tolerance,
				})
			} else {
				Err(format!(
					"unknown checker '{name}'; the built-ins are {}",
					BUILTIN_CHECKERS.join(", ")
				))
			}
		};
		match self {
			CheckMethod::Builtin(name) => builtin(name, None),
			CheckMethod::Detailed(spec) => {
				let kinds: Vec<&str> = [
					spec.builtin.as_ref().map(|_| "builtin"),
					spec.rhai.as_ref().map(|_| "rhai"),
					spec.python.as_ref().map(|_| "python"),
					spec.function.as_ref().map(|_| "function"),
				]
				.into_iter()
				.flatten()
				.collect();
				if kinds.len() != 1 {
					return Err(format!(
						"a check must name exactly one of builtin, rhai, python, function; this one names {}",
						if kinds.is_empty() {
							"none".to_string()
						} else {
							kinds.join(" and ")
						}
					));
				}
				if spec.tolerance.is_some() && spec.builtin.as_deref() != Some("approx") {
					return Err("tolerance only applies to builtin = \"approx\"".to_string());
				}
				if let Some(name) = &spec.builtin {
					builtin(name, spec.tolerance)
				} else if let Some(expr) = &spec.rhai {
					Ok(Check::Rhai(expr.clone()))
				} else if let Some(script) = &spec.python {
					Ok(Check::Python(script.clone()))
				} else {
					Ok(Check::Function(spec.function.clone().unwrap_or_default()))
				}
			}
		}
	}
}

/// Oracle — how to determine the expected output for generated inputs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Oracle {
	/// Teacher's reference implementation file. Same function name, compare outputs.
	#[serde(default)]
	pub reference: Option<String>,
	/// Rhai expression computing expected value from generated args.
	#[serde(default)]
	pub rhai: Option<String>,
	/// Built-in checker name (just verifies a property, no expected value).
	#[serde(default)]
	pub check: Option<String>,
}

/// A template: concrete cases from written samples and from random draws.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parametrize {
	/// The parameters in call order, each `{ name = "rule" }`.
	#[serde(default, deserialize_with = "call_order")]
	pub args: Vec<Param>,
	/// Inputs written out, each a list of values in call order, like a fixed case's
	/// `args`. A sample is an input, never an answer.
	#[serde(default)]
	pub samples: Vec<Vec<Value>>,
	/// Random draws from the rules in `args`.
	#[serde(default)]
	pub random: Option<Random>,
	/// How to determine the expected output.
	#[serde(default)]
	pub oracle: Oracle,
	/// Refused: moved to `[random]`.
	#[serde(default, skip_serializing)]
	pub count: Option<Value>,
	/// Refused: moved to `[random]`.
	#[serde(default, skip_serializing)]
	pub seed: Option<Value>,
}

impl Parametrize {
	/// Everything that decides the inputs.
	pub fn inputs(&self) -> Inputs {
		Inputs {
			args: self.args.clone(),
			samples: self.samples.clone(),
			random: self.random.clone(),
		}
	}
}

/// Everything that decides a template's inputs, and nothing that judges them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
	pub args: Vec<Param>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub samples: Vec<Vec<Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub random: Option<Random>,
}

impl Inputs {
	/// The parameter names, in call order.
	pub fn names(&self) -> Vec<&str> {
		self.args.iter().map(|p| p.name.as_str()).collect()
	}
}

/// One generated parameter, written `{ name = "rule" }`. Its place in `args` is its place
/// in the call; its name binds it in a Rhai oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
	pub name: String,
	pub rule: String,
}

impl Serialize for Param {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut map = serializer.serialize_map(Some(1))?;
		map.serialize_entry(&self.name, &self.rule)?;
		map.end()
	}
}

impl<'de> Deserialize<'de> for Param {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let mut entries = BTreeMap::<String, String>::deserialize(deserializer)?.into_iter();
		match (entries.next(), entries.next()) {
			(Some((name, rule)), None) => Ok(Param { name, rule }),
			_ => Err(de::Error::custom(
				"a parameter is one `name = \"rule\"` pair, e.g. { low = \"int(0, 9)\" }: give each parameter its own entry",
			)),
		}
	}
}

/// `args` as a list in call order. The table it used to be has no order, so it is refused
/// with the fix rather than bound in some order the teacher never chose.
fn call_order<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Param>, D::Error> {
	struct CallOrder;

	impl<'de> Visitor<'de> for CallOrder {
		type Value = Vec<Param>;

		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			f.write_str("a list of parameters in call order, each { name = \"rule\" }")
		}

		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Param>, A::Error> {
			let mut params = Vec::new();
			while let Some(param) = seq.next_element()? {
				params.push(param);
			}
			Ok(params)
		}

		fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Vec<Param>, M::Error> {
			let mut names = Vec::new();
			while let Some((name, _)) = map.next_entry::<String, de::IgnoredAny>()? {
				names.push(name);
			}
			Err(de::Error::custom(format!(
				"args is now a list in call order, one `name = \"rule\"` per parameter: write a [[cases.parametrize.args]] block for each of {}, in the order the function takes them",
				names.join(", ")
			)))
		}
	}

	deserializer.deserialize_any(CallOrder)
}

/// Random draws from a template's rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Random {
	/// How many draws.
	pub count: usize,
	/// Omitted means 0.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub seed: Option<Seed>,
}

/// A declared seed: a number, or `"random"` to draw one and record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seed {
	Fixed(u64),
	Random,
}

impl Serialize for Seed {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Seed::Fixed(n) => serializer.serialize_u64(*n),
			Seed::Random => serializer.serialize_str("random"),
		}
	}
}

impl<'de> Deserialize<'de> for Seed {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct SeedVisitor;

		impl Visitor<'_> for SeedVisitor {
			type Value = Seed;

			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a seed: a whole number, 0 or more, or \"random\"")
			}

			fn visit_u64<E: de::Error>(self, v: u64) -> Result<Seed, E> {
				Ok(Seed::Fixed(v))
			}

			fn visit_i64<E: de::Error>(self, v: i64) -> Result<Seed, E> {
				u64::try_from(v).map(Seed::Fixed).map_err(|_| {
					E::custom(format!(
						"seed {v} is negative: a seed is a whole number, 0 or more, or \"random\""
					))
				})
			}

			fn visit_str<E: de::Error>(self, v: &str) -> Result<Seed, E> {
				match v {
					"random" => Ok(Seed::Random),
					_ => Err(E::custom(format!(
						"seed \"{v}\" is not a seed: write a whole number, 0 or more, or \"random\""
					))),
				}
			}
		}

		deserializer.deserialize_any(SeedVisitor)
	}
}

/// What a call runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
	/// A callable on the student module, found by today's fuzzy lookup (P-673 owns it).
	Function { name: String },
	/// A method of a live object an earlier student call produced in the same unit.
	Method { object: String, name: String },
	/// An attribute of such an object.
	Attribute { object: String, name: String },
	/// A function exported by a teacher module.
	Teacher { name: String },
}

impl Target {
	/// Whether the call runs the student's code — which decides who owns its failures.
	pub fn is_student(&self) -> bool {
		!matches!(self, Target::Teacher { .. })
	}
}

/// A test case (`[[cases]]`) or a scenario step (`[[scenarios.steps]]`).
///
/// A case runs in its own process and working directory; a step shares its scenario's.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
	pub name: String,

	/// On a scenario step: store the step's value under this id for later steps. Refused
	/// on a case — nothing outlives a case.
	#[serde(default)]
	pub id: Option<String>,

	/// Call a student function. Defaults to `[meta] function`.
	#[serde(default)]
	pub function: Option<String>,

	/// Call a method on `object`.
	#[serde(default)]
	pub method: Option<String>,

	/// Read an attribute of `object`.
	#[serde(default)]
	pub attribute: Option<String>,

	/// The id of a live object an earlier student call produced in this unit.
	#[serde(default)]
	pub object: Option<String>,

	/// Run the student file as `__main__` instead of calling a function.
	#[serde(default)]
	pub script: bool,

	/// Arguments for the call. A string `"$id"` is replaced by the value named `id`;
	/// `"$$..."` passes a literal leading `$`.
	#[serde(default)]
	pub args: Vec<Value>,

	/// What the call reads from stdin.
	#[serde(default)]
	pub stdin: Option<String>,

	/// Expected return value.
	#[serde(default)]
	pub expect: Option<Value>,

	/// Expected exception type; a subclass also matches.
	#[serde(default)]
	pub expect_error: Option<String>,

	/// Expected stdout of the call, compared exactly (in script mode, by `check`).
	#[serde(default)]
	pub expected_stdout: Option<String>,

	/// Files the call must leave in the working directory, with their exact contents.
	#[serde(default)]
	pub expect_files: BTreeMap<String, String>,

	/// How to check the value. Defaults to exact comparison against `expect`.
	#[serde(default)]
	pub check: Option<CheckMethod>,

	/// Timeout in seconds for this call.
	#[serde(default)]
	pub timeout: Option<u64>,

	/// Generate concrete cases from this one.
	#[serde(default)]
	pub parametrize: Option<Parametrize>,
}

impl TestCase {
	/// The call this case makes, or `None` for a script run. Assumes a validated spec.
	pub fn target(&self, default_function: Option<&str>) -> Option<Target> {
		if self.script {
			return None;
		}
		let object = || self.object.clone().unwrap_or_default();
		if let Some(name) = &self.method {
			Some(Target::Method {
				object: object(),
				name: name.clone(),
			})
		} else if let Some(name) = &self.attribute {
			Some(Target::Attribute {
				object: object(),
				name: name.clone(),
			})
		} else {
			self.function
				.as_deref()
				.or(default_function)
				.map(|name| Target::Function {
					name: name.to_string(),
				})
		}
	}
}

/// Metadata for a test spec file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestMeta {
	/// The grading item this spec provides evidence for.
	pub name: String,

	/// Student file to test (suffix match, e.g. "Lab5_1.py"). P-673 owns the matching.
	pub file: String,

	/// Language of the student code. Only `python` is supported.
	pub language: String,

	/// Default function for cases and steps that name no target.
	#[serde(default)]
	pub function: Option<String>,

	/// Refused: compiled languages are not supported.
	#[serde(default)]
	pub compile: Option<String>,

	/// Teacher modules, loaded fresh in every unit. Their exports are names in scope.
	#[serde(default)]
	pub imports: Vec<String>,

	/// Teacher files copied into every unit's working directory, at the same relative path.
	#[serde(default)]
	pub data_files: Vec<String>,

	/// Refused: every case now has its own process.
	#[serde(default)]
	pub copy_refs: Option<bool>,

	/// Extra packages allowed in student code (beyond safe stdlib).
	#[serde(default)]
	pub allowed_imports: Vec<String>,
}

/// A setup call. Top-level setup runs at the start of every case and every scenario;
/// scenario setup runs once at the start of its scenario. It is not scored.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SetupStep {
	/// The name its value is stored under, for `$id` and `object`.
	pub id: String,

	/// Call a student function.
	#[serde(default)]
	pub function: Option<String>,

	/// Call a method on `object`.
	#[serde(default)]
	pub method: Option<String>,

	/// The id of a live object an earlier student call produced in this unit.
	#[serde(default)]
	pub object: Option<String>,

	/// Call a function exported by a teacher module.
	#[serde(default)]
	pub teacher: Option<String>,

	/// Arguments for the call. May contain `$ref` strings.
	#[serde(default)]
	pub args: Vec<Value>,

	/// Timeout in seconds for this call.
	#[serde(default)]
	pub timeout: Option<u64>,

	/// Refused: generated inputs belong to P-675.
	#[serde(default)]
	pub file: Option<String>,
}

impl SetupStep {
	/// The call this step makes. Assumes a validated spec.
	pub fn target(&self) -> Target {
		if let Some(name) = &self.teacher {
			Target::Teacher { name: name.clone() }
		} else if let Some(name) = &self.method {
			Target::Method {
				object: self.object.clone().unwrap_or_default(),
				name: name.clone(),
			}
		} else {
			Target::Function {
				name: self.function.clone().unwrap_or_default(),
			}
		}
	}
}

/// A shared-state scenario: steps run in order in one process and one working directory.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
	pub name: String,

	/// Default timeout in seconds for this scenario's setup and steps.
	#[serde(default)]
	pub timeout: Option<u64>,

	/// Runs once, at the start of the scenario, after the top-level setup.
	#[serde(default)]
	pub setup: Vec<SetupStep>,

	/// Each step is judged and reported as its own case, `"<scenario> / <step>"`.
	#[serde(default)]
	pub steps: Vec<TestCase>,
}

impl Scenario {
	/// The name a step's result is reported under.
	pub fn step_name(&self, step: &TestCase) -> String {
		format!("{} / {}", self.name, step.name)
	}
}

/// A complete test specification (one TOML file).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestSpec {
	pub meta: TestMeta,
	/// Constants — student module globals, and names in scope for `$ref`.
	#[serde(default)]
	pub vars: BTreeMap<String, Value>,
	/// Runs at the start of every case and every scenario, inside its process.
	#[serde(default)]
	pub setup: Vec<SetupStep>,
	/// Independent cases.
	#[serde(default)]
	pub cases: Vec<TestCase>,
	/// Shared-state scenarios.
	#[serde(default)]
	pub scenarios: Vec<Scenario>,
	/// Optional lint-based style scoring.
	#[serde(default)]
	pub lint: Option<LintConfig>,
	/// The directory relative teacher paths are resolved against. Set by the loader.
	#[serde(skip)]
	pub dir: PathBuf,
}
