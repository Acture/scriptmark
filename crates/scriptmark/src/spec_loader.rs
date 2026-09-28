use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::checker::rhai_checker;

use crate::models::{
	AssignmentConfig, Check, CourseConfig, Scenario, SetupStep, Target, TestCase, TestSpec,
};

/// Load and validate a test specification from a TOML file.
///
/// Relative teacher paths (`imports`, Python checker scripts, oracle `reference`) are
/// resolved against the file's directory and made absolute; `data_files` stay relative,
/// because their relative path is also where they are staged.
pub fn load_spec(path: &Path) -> Result<TestSpec, SpecError> {
	let content =
		std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
	load_spec_str(&content, spec_dir(path)).map_err(|e| e.at(path))
}

/// The directory a spec's relative paths mean. `Path::new("spec.toml").parent()` is
/// `Some("")`, not `None`, so a bare filename needs saying.
fn spec_dir(path: &Path) -> &Path {
	path.parent()
		.filter(|p| !p.as_os_str().is_empty())
		.unwrap_or(Path::new("."))
}

/// Load and validate a test specification from TOML text, resolving paths against `dir`.
pub fn load_spec_str(content: &str, dir: &Path) -> Result<TestSpec, SpecError> {
	let mut spec: TestSpec =
		toml::from_str(content).map_err(|e| SpecError::ParseError(PathBuf::new(), e))?;
	spec.dir = std::path::absolute(dir).map_err(|e| SpecError::IoError(dir.to_path_buf(), e))?;
	resolve_paths(&mut spec);
	let problems = validate(&spec);
	if problems.is_empty() {
		Ok(spec)
	} else {
		Err(SpecError::Invalid {
			path: PathBuf::new(),
			name: spec.meta.name,
			problems,
		})
	}
}

/// Load all test specifications from a directory (*.toml files). Every invalid file is
/// reported, not just the first.
pub fn load_specs_from_dir(dir: &Path) -> Result<Vec<TestSpec>, SpecError> {
	if !dir.is_dir() {
		return Err(SpecError::NotADirectory(dir.to_path_buf()));
	}

	let mut entries: Vec<_> = std::fs::read_dir(dir)
		.map_err(|e| SpecError::IoError(dir.to_path_buf(), e))?
		.filter_map(|e| e.ok())
		.filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
		.collect();

	// Deterministic ordering
	entries.sort_by_key(|e| e.path());

	let mut specs = Vec::new();
	let mut errors = Vec::new();
	for entry in entries {
		match load_spec(&entry.path()) {
			Ok(spec) => specs.push(spec),
			Err(e) => errors.push(e),
		}
	}
	match errors.len() {
		0 => Ok(specs),
		1 => Err(errors.remove(0)),
		_ => Err(SpecError::Several(errors)),
	}
}

fn absolute_in(dir: &Path, path: &str) -> String {
	let p = Path::new(path);
	if p.is_absolute() {
		path.to_string()
	} else {
		dir.join(p).to_string_lossy().into_owned()
	}
}

fn resolve_paths(spec: &mut TestSpec) {
	let dir = spec.dir.clone();
	for import in &mut spec.meta.imports {
		*import = absolute_in(&dir, import);
	}
	let cases = spec
		.cases
		.iter_mut()
		.chain(spec.scenarios.iter_mut().flat_map(|s| s.steps.iter_mut()));
	for case in cases {
		if let Some(crate::models::CheckMethod::Detailed(check)) = &mut case.check
			&& let Some(script) = &mut check.python
		{
			*script = absolute_in(&dir, script);
		}
		if let Some(param) = &mut case.parametrize
			&& let Some(reference) = &mut param.oracle.reference
		{
			*reference = absolute_in(&dir, reference);
		}
	}
}

/// The name a function checker's `stdout` parameter always receives: the call's output.
pub const RESERVED: &str = "stdout";

/// The longest timeout a spec or the command line may ask for.
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// What the harness keeps of a call's stdout; an expectation longer than this can never match.
pub const STDOUT_LIMIT: usize = 64 * 1024;

/// Every static problem with a spec: shape rules the type system cannot express, paths
/// that do not exist, and configurations that would otherwise run along a default path.
///
/// `prepare` runs this again, so a `TestSpec` built by hand cannot skip it.
pub fn validate(spec: &TestSpec) -> Vec<String> {
	let mut v = Validator {
		spec,
		problems: Vec::new(),
	};
	v.meta();
	v.body();
	v.problems
}

struct Validator<'a> {
	spec: &'a TestSpec,
	problems: Vec<String>,
}

/// Ids bound so far in one unit, and which of them a student call produced.
#[derive(Clone, Default)]
struct Bound {
	all: BTreeSet<String>,
	student: BTreeSet<String>,
}

impl Bound {
	fn bind(&mut self, id: &str, target: &Target) {
		self.all.insert(id.to_string());
		if target.is_student() {
			self.student.insert(id.to_string());
		}
	}
}

impl Validator<'_> {
	fn problem(&mut self, at: &str, message: impl AsRef<str>) {
		self.problems.push(format!("{at}: {}", message.as_ref()));
	}

	fn meta(&mut self) {
		let meta = &self.spec.meta;
		if meta.language != "python" {
			self.problem(
				"[meta]",
				format!(
					"language '{}' is not supported; only python is",
					meta.language
				),
			);
		}
		if meta.compile.is_some() {
			self.problem("[meta]", "compile is not supported");
		}
		if meta.copy_refs.is_some() {
			self.problem(
				"[meta]",
				"copy_refs was removed: every case runs in its own process, and a scenario shares state on purpose",
			);
		}
		if meta.file.trim().is_empty() {
			self.problem("[meta]", "file must name the student file to test");
		}
		for import in &meta.imports {
			if !Path::new(import).is_file() {
				self.problem(
					"[meta]",
					format!("teacher module '{import}' does not exist"),
				);
			}
		}
		for data in &meta.data_files {
			if !is_contained(data) {
				self.problem(
					"[meta]",
					format!("data file '{data}' must be a relative path without '..'"),
				);
			} else if !self.spec.dir.join(data).exists() {
				self.problem("[meta]", format!("data file '{data}' does not exist"));
			}
		}
		if self.spec.vars.contains_key(RESERVED) {
			self.problem(
				"[vars]",
				format!(
					"'{RESERVED}' is reserved: function checkers receive the call's output under it"
				),
			);
		}
		for (name, value) in &self.spec.vars {
			if contains_null(value) {
				self.problem(
					"[vars]",
					format!("'{name}' contains a value TOML cannot hold (inf or nan?)"),
				);
			}
		}
	}

	fn body(&mut self) {
		let spec = self.spec;
		if spec.cases.is_empty() && spec.scenarios.is_empty() {
			self.problem("spec", "has no [[cases]] and no [[scenarios]]");
		}

		let mut top = Bound::default();
		for step in &spec.setup {
			self.setup(step, &mut top, &format!("setup '{}'", step.id));
		}
		if !spec.setup.is_empty()
			&& let Some(case) = spec.cases.iter().find(|c| c.script)
		{
			self.problem(
				&format!("case '{}'", case.name),
				"a script case cannot run alongside top-level [[setup]], which calls into the student module",
			);
		}

		let mut names = BTreeSet::new();
		for case in &spec.cases {
			let at = format!("case '{}'", case.name);
			self.case(case, &top, false, &at);
			match &case.parametrize {
				Some(param) => {
					for i in 0..param.count {
						self.unique_name(&mut names, format!("{} [{i}]", case.name));
					}
				}
				None => self.unique_name(&mut names, case.name.clone()),
			}
		}
		for scenario in &spec.scenarios {
			self.scenario(scenario, &top, &mut names);
		}
	}

	fn timeout(&mut self, timeout: Option<u64>, at: &str) {
		if timeout.is_some_and(|t| !(1..=MAX_TIMEOUT_SECS).contains(&t)) {
			self.problem(
				at,
				format!("timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"),
			);
		}
	}

	fn unique_name(&mut self, names: &mut BTreeSet<String>, name: String) {
		if !names.insert(name.clone()) {
			self.problem(
				&format!("case '{name}'"),
				"two cases have this name; results could not be told apart",
			);
		}
	}

	fn scenario(&mut self, scenario: &Scenario, top: &Bound, names: &mut BTreeSet<String>) {
		let at = format!("scenario '{}'", scenario.name);
		if scenario.name.trim().is_empty() {
			self.problem(&at, "needs a name");
		}
		if scenario.steps.is_empty() {
			self.problem(&at, "has no steps");
		}
		self.timeout(scenario.timeout, &at);
		let mut bound = top.clone();
		for step in &scenario.setup {
			self.setup(step, &mut bound, &format!("{at} setup '{}'", step.id));
		}
		for step in &scenario.steps {
			let step_at = format!("{at} step '{}'", step.name);
			self.case(step, &bound, true, &step_at);
			if let Some(id) = &step.id {
				if bound.all.contains(id) || self.spec.vars.contains_key(id) {
					self.problem(&step_at, format!("id '{id}' is already bound"));
				}
				if id == RESERVED {
					self.problem(
						&step_at,
						format!("id '{RESERVED}' is reserved for the call's output"),
					);
				}
				if let Some(target) = step.target(self.spec.meta.function.as_deref()) {
					bound.bind(id, &target);
				}
			}
			self.unique_name(names, scenario.step_name(step));
		}
	}

	fn setup(&mut self, step: &SetupStep, bound: &mut Bound, at: &str) {
		if step.id.trim().is_empty() {
			self.problem(at, "needs an id");
		}
		if bound.all.contains(&step.id) || self.spec.vars.contains_key(&step.id) {
			self.problem(at, format!("id '{}' is already bound", step.id));
		}
		if step.id == RESERVED {
			self.problem(
				at,
				format!("id '{RESERVED}' is reserved for the call's output"),
			);
		}
		if step.args.iter().any(contains_null) {
			self.problem(
				at,
				"args contain a value TOML cannot pass (inf or nan?); put it in a teacher module and pass '$name'",
			);
		}
		if step.file.is_some() {
			self.problem(
				at,
				"setup.file is not supported: put fixed data in [vars], data_files or a teacher module; generated inputs arrive with P-675",
			);
			return;
		}
		self.timeout(step.timeout, at);
		let targets = [
			step.function.is_some(),
			step.method.is_some(),
			step.teacher.is_some(),
		]
		.iter()
		.filter(|t| **t)
		.count();
		if targets != 1 {
			self.problem(
				at,
				"must name exactly one of function, method (with object) or teacher",
			);
			return;
		}
		match (&step.method, &step.object) {
			(Some(_), None) => self.problem(at, "method needs object = \"<id>\""),
			(None, Some(_)) => self.problem(at, "object only applies to method"),
			(Some(_), Some(object)) if !bound.student.contains(object) => self.problem(
				at,
				format!(
					"object '{object}' is not an id an earlier student call produced in this unit"
				),
			),
			_ => {}
		}
		let target = step.target();
		if !target.is_student() {
			for name in refs(&step.args) {
				if bound.student.contains(&name) {
					self.problem(
						at,
						format!(
							"a teacher call cannot take '${name}', a value student code produced — its failures would be blamed on the teacher"
						),
					);
				}
			}
		}
		bound.bind(&step.id, &target);
	}

	fn case(&mut self, case: &TestCase, bound: &Bound, is_step: bool, at: &str) {
		if case.name.trim().is_empty() {
			self.problem(at, "needs a name");
		}
		if !is_step && case.id.is_some() {
			self.problem(
				at,
				"id only applies to scenario steps: nothing outlives an independent case",
			);
		}
		self.timeout(case.timeout, at);
		if case.args.iter().any(contains_null) {
			self.problem(
				at,
				"args contain a value TOML cannot pass (inf or nan?); put it in a teacher module and pass '$name'",
			);
		}
		if let Some(expected) = &case.expected_stdout
			&& expected.len() > STDOUT_LIMIT
		{
			self.problem(
				at,
				format!(
					"expected_stdout is longer than the {} KiB of output kept",
					STDOUT_LIMIT / 1024
				),
			);
		}

		if case.script {
			self.script_case(case, is_step, at);
		} else {
			self.call_case(case, bound, at);
		}

		if case.expect.is_some() && case.expect_error.is_some() {
			self.problem(at, "expect and expect_error contradict each other");
		}
		if case.check.is_some() && case.expect_error.is_some() {
			self.problem(
				at,
				"check and expect_error contradict each other: a raised exception has no value to check",
			);
		}
		if let Some(expect) = &case.expect
			&& contains_null(expect)
		{
			self.problem(
				at,
				"expect contains a value TOML cannot hold (inf or nan?); use check = { builtin = \"approx\" } or rhai",
			);
		}
		for path in case.expect_files.keys() {
			if !is_contained(path) {
				self.problem(
					at,
					format!("expect_files path '{path}' must be relative, without '..'"),
				);
			}
		}

		let oracle_expects = case
			.parametrize
			.as_ref()
			.is_some_and(|p| p.oracle.reference.is_some() || p.oracle.rhai.is_some());
		if let Some(method) = &case.check {
			match method.resolve() {
				Err(e) => self.problem(at, e),
				Ok(check) => self.check(&check, case, oracle_expects, at),
			}
		}
		if let Some(param) = &case.parametrize {
			if is_step {
				self.problem(at, "a scenario step cannot be parametrized");
			}
			self.parametrize(case, param, at);
		}

		let judged = case.expect.is_some()
			|| case.expect_error.is_some()
			|| case.expected_stdout.is_some()
			|| !case.expect_files.is_empty()
			|| case.check.is_some()
			|| oracle_expects
			|| case
				.parametrize
				.as_ref()
				.is_some_and(|p| p.oracle.check.is_some());
		if !judged {
			self.problem(
				at,
				"declares nothing to judge: add expect, expect_error, expected_stdout, expect_files or check",
			);
		}
	}

	fn script_case(&mut self, case: &TestCase, is_step: bool, at: &str) {
		if is_step {
			self.problem(at, "a scenario step cannot run the file as a script");
		}
		let refused = [
			(case.function.is_some(), "function"),
			(case.method.is_some(), "method"),
			(case.attribute.is_some(), "attribute"),
			(case.object.is_some(), "object"),
			(!case.args.is_empty(), "args"),
			(case.expect.is_some(), "expect"),
			(case.parametrize.is_some(), "parametrize"),
			(
				matches!(
					case.check.as_ref().map(|c| c.resolve()),
					Some(Ok(Check::Function(_)))
				),
				"check = { function } (it judges a returned value; use a builtin, rhai or python on the output)",
			),
		];
		for (present, field) in refused {
			if present {
				self.problem(
					at,
					format!(
						"{field} does not apply to a script case, which runs the file as __main__"
					),
				);
			}
		}
	}

	fn call_case(&mut self, case: &TestCase, bound: &Bound, at: &str) {
		let named = [
			case.function.is_some(),
			case.method.is_some(),
			case.attribute.is_some(),
		]
		.iter()
		.filter(|t| **t)
		.count();
		if named > 1 {
			self.problem(at, "name at most one of function, method, attribute");
			return;
		}
		let on_object = case.method.is_some() || case.attribute.is_some();
		match (&case.object, on_object) {
			(None, true) => self.problem(at, "method and attribute need object = \"<id>\""),
			(Some(_), false) => self.problem(at, "object only applies to method or attribute"),
			(Some(object), true) if !bound.student.contains(object) => self.problem(
				at,
				format!(
					"object '{object}' is not an id an earlier student call produced in this unit"
				),
			),
			_ => {}
		}
		if case.attribute.is_some() && !case.args.is_empty() {
			self.problem(at, "reading an attribute takes no args");
		}
		if named == 0 && self.spec.meta.function.is_none() {
			self.problem(
				at,
				"calls nothing: name a function (or set [meta] function), or set script = true",
			);
		}
	}

	/// A property built-in beside `expect` must hold of `expect` too: the value has to equal
	/// it and pass the check, so an `expect` that fails the check fails every student.
	fn both_can_hold(&mut self, check: &Check, expect: Option<&Value>, at: &str) {
		let (Check::Builtin { name, tolerance }, Some(expect)) = (check, expect) else {
			return;
		};
		if check.needs_expectation() {
			return;
		}
		let checker = crate::checker::builtin::resolve_builtin(name, *tolerance)
			.expect("a resolved built-in exists");
		let input = crate::checker::CheckInput {
			result: expect.clone(),
			expected: Value::Null,
			context: Value::Null,
		};
		if let Ok(out) = checker.check(&input)
			&& !out.pass
		{
			self.problem(
				at,
				format!(
					"expect fails the {name} check itself, so no value can pass both: {}",
					out.message
				),
			);
		}
	}

	fn check(&mut self, check: &Check, case: &TestCase, oracle_expects: bool, at: &str) {
		// A property built-in judges a list; a script's value is the text it printed.
		if case.script
			&& let Check::Builtin { name, .. } = check
			&& !check.needs_expectation()
		{
			self.problem(
				at,
				format!(
					"the {name} checker judges a list, but a script's value is its printed text"
				),
			);
		}
		if !case.script {
			self.both_can_hold(check, case.expect.as_ref(), at);
		}
		if check.needs_expectation() {
			let has = if case.script {
				case.expected_stdout.is_some()
			} else {
				case.expect.is_some() || oracle_expects
			};
			if !has {
				self.problem(
					at,
					if case.script {
						"this checker compares against expected_stdout, which is missing"
					} else {
						"this checker compares against expect, which is missing"
					},
				);
			}
		}
		// The shape a comparison needs: a wrong one fails every student for a teacher's typo.
		if let Check::Builtin { name, .. } = check {
			let expected = if case.script {
				case.expected_stdout.clone().map(serde_json::Value::String)
			} else {
				case.expect.clone()
			};
			let wants = match name.as_str() {
				"approx" => Some(("a number", expected.as_ref().is_none_or(Value::is_number))),
				"set_eq" => Some(("an array", expected.as_ref().is_none_or(Value::is_array))),
				"text" => Some(("a string", expected.as_ref().is_none_or(Value::is_string))),
				_ => None,
			};
			if let Some((shape, false)) = wants {
				self.problem(
					at,
					format!("the {name} checker needs {shape} to compare against"),
				);
			}
		}
		match check {
			Check::Rhai(expr) => {
				if let Err(e) = rhai_checker::compile(expr, &rhai_checker::VARIABLES) {
					self.problem(at, format!("rhai check does not compile: {e}"));
				}
			}
			Check::Python(script) if !Path::new(script).is_file() => {
				self.problem(at, format!("checker script '{script}' does not exist"));
			}
			Check::Function(name) if name.trim().is_empty() => {
				self.problem(at, "check.function needs a name");
			}
			_ => {}
		}
	}

	fn parametrize(&mut self, case: &TestCase, param: &crate::models::Parametrize, at: &str) {
		if param.count == 0 {
			self.problem(at, "parametrize.count must be at least 1");
		}
		if !case.args.is_empty() {
			self.problem(at, "a parametrized case generates its args; remove args");
		}
		let oracle = &param.oracle;
		let kinds = [
			oracle.reference.is_some(),
			oracle.rhai.is_some(),
			oracle.check.is_some(),
		]
		.iter()
		.filter(|k| **k)
		.count();
		if kinds > 1 {
			self.problem(at, "an oracle names exactly one of reference, rhai, check");
		}
		if (oracle.reference.is_some() || oracle.rhai.is_some()) && case.expect.is_some() {
			self.problem(
				at,
				"expect conflicts with an oracle that computes the expectation",
			);
		}
		if (oracle.reference.is_some() || oracle.rhai.is_some()) && case.expect_error.is_some() {
			self.problem(
				at,
				"expect_error conflicts with an oracle that computes a returned value",
			);
		}
		if oracle.check.is_some() && case.check.is_some() {
			self.problem(
				at,
				"check conflicts with oracle.check, which would replace it",
			);
		}
		if oracle.check.is_some() && case.expect_error.is_some() {
			self.problem(
				at,
				"expect_error conflicts with oracle.check: a raised exception has no value to check",
			);
		}
		if case.attribute.is_some() {
			self.problem(
				at,
				"an attribute takes no args, so it cannot be parametrized",
			);
		}
		if oracle.reference.is_some() && case.method.is_some() {
			self.problem(
				at,
				"a reference oracle calls a function; it cannot answer for a method call",
			);
		}
		if let Some(name) = &oracle.check {
			match crate::models::CheckMethod::Builtin(name.clone()).resolve() {
				Err(e) => self.problem(at, format!("oracle.check: {e}")),
				Ok(check) if check.needs_expectation() => self.problem(
					at,
					format!(
						"oracle.check '{name}' compares against an expectation, which a check oracle does not provide"
					),
				),
				Ok(check) => self.both_can_hold(&check, case.expect.as_ref(), at),
			}
		}
		if let Some(expr) = &oracle.rhai {
			let names: Vec<&str> = param.args.keys().map(String::as_str).collect();
			if let Err(e) = rhai_checker::compile(expr, &names) {
				self.problem(at, format!("rhai oracle does not compile: {e}"));
			}
		}
		if let Some(reference) = &oracle.reference
			&& !Path::new(reference).is_file()
		{
			self.problem(
				at,
				format!("reference implementation '{reference}' does not exist"),
			);
		}
	}
}

/// A relative path that cannot climb out of the directory it is joined to.
fn is_contained(path: &str) -> bool {
	!path.trim().is_empty()
		&& Path::new(path)
			.components()
			.all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// TOML has no null, so a null in a parsed value is a non-finite float serde_json lost.
fn contains_null(value: &Value) -> bool {
	match value {
		Value::Null => true,
		Value::Array(items) => items.iter().any(contains_null),
		Value::Object(map) => map.values().any(contains_null),
		_ => false,
	}
}

/// The `$name` references in a list of arguments (`$$` escapes a literal `$`).
pub fn refs(args: &[Value]) -> Vec<String> {
	fn walk(value: &Value, out: &mut Vec<String>) {
		match value {
			Value::String(s) if s.starts_with('$') && !s.starts_with("$$") => {
				out.push(s[1..].to_string())
			}
			Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
			Value::Object(map) => map.values().for_each(|v| walk(v, out)),
			_ => {}
		}
	}
	let mut out = Vec::new();
	args.iter().for_each(|v| walk(v, &mut out));
	out
}

/// Load course configuration from course.toml.
pub fn load_course_config(path: &Path) -> Result<CourseConfig, SpecError> {
	let content =
		std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
	let config: CourseConfig =
		toml::from_str(&content).map_err(|e| SpecError::ParseError(path.to_path_buf(), e))?;
	Ok(config)
}

/// Load assignment configuration from assignment.toml.
pub fn load_assignment_config(path: &Path) -> Result<AssignmentConfig, SpecError> {
	let content =
		std::fs::read_to_string(path).map_err(|e| SpecError::IoError(path.to_path_buf(), e))?;
	let config: AssignmentConfig =
		toml::from_str(&content).map_err(|e| SpecError::ParseError(path.to_path_buf(), e))?;
	Ok(config)
}

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
	#[error("not a directory: {0}")]
	NotADirectory(PathBuf),
	#[error("IO error reading {0}: {1}")]
	IoError(PathBuf, std::io::Error),
	#[error("TOML parse error in {0}: {1}")]
	ParseError(PathBuf, toml::de::Error),
	#[error("test spec '{name}' ({}) is invalid:\n  {}", path.display(), problems.join("\n  "))]
	Invalid {
		path: PathBuf,
		name: String,
		problems: Vec<String>,
	},
	#[error("{}", .0.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))]
	Several(Vec<SpecError>),
}

impl SpecError {
	fn at(self, path: &Path) -> Self {
		match self {
			SpecError::ParseError(_, e) => SpecError::ParseError(path.to_path_buf(), e),
			SpecError::Invalid { name, problems, .. } => SpecError::Invalid {
				path: path.to_path_buf(),
				name,
				problems,
			},
			other => other,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_load_spec() {
		let dir = tempfile::tempdir().unwrap();
		let spec_path = dir.path().join("test_larger.toml");
		std::fs::write(
			&spec_path,
			r#"
[meta]
name = "find_larger_number"
file = "Lab5_1.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "3 < 5"
args = [3, 5]
expect = 5

[[cases]]
name = "negative"
args = [-3, -2]
expect = -2

[[cases]]
name = "raises TypeError"
args = ["a", 1]
expect_error = "TypeError"
"#,
		)
		.unwrap();

		let spec = load_spec(&spec_path).unwrap();
		assert_eq!(spec.meta.name, "find_larger_number");
		assert_eq!(spec.meta.language, "python");
		assert_eq!(spec.meta.function.as_deref(), Some("find_larger_number"));
		assert_eq!(spec.cases.len(), 3);
		assert_eq!(spec.cases[0].name, "3 < 5");
		assert_eq!(spec.cases[2].expect_error.as_deref(), Some("TypeError"));
		assert!(spec.dir.is_absolute());
	}

	#[test]
	fn test_load_course_config() {
		let dir = tempfile::tempdir().unwrap();
		let config_path = dir.path().join("course.toml");
		std::fs::write(
			&config_path,
			r#"
[course]
name = "GEEC Python"
language = "python"

[grading]
template = "sqrt"
lower = 60
upper = 100
"#,
		)
		.unwrap();

		let config = load_course_config(&config_path).unwrap();
		assert_eq!(config.course.name, "GEEC Python");
		assert_eq!(config.course.language, "python");
	}

	#[test]
	fn test_load_parametrized_spec() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir(dir.path().join("solutions")).unwrap();
		std::fs::write(dir.path().join("solutions/lab5.py"), "").unwrap();
		let path = dir.path().join("test_param.toml");
		std::fs::write(
			&path,
			r#"
[meta]
name = "random_max"
file = "lab5.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "random pairs"

[cases.parametrize]
count = 20
seed = 42

[cases.parametrize.args]
a = "int(-100, 100)"
b = "int(-100, 100)"

[cases.parametrize.oracle]
reference = "solutions/lab5.py"
"#,
		)
		.unwrap();

		let spec = load_spec(&path).unwrap();
		assert_eq!(spec.cases.len(), 1);
		let param = spec.cases[0].parametrize.as_ref().unwrap();
		assert_eq!(param.count, 20);
		assert_eq!(param.seed, Some(42));
		assert_eq!(param.args.len(), 2);
		let reference = param.oracle.reference.as_deref().unwrap();
		assert!(
			Path::new(reference).is_absolute(),
			"reference is resolved against the spec, not the grader's cwd"
		);
	}

	const META: &str =
		"[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\nfunction = \"f\"\n";

	/// The problems a spec body produces, with the common `[meta]` prepended.
	fn problems(body: &str) -> Vec<String> {
		let dir = tempfile::tempdir().unwrap();
		match load_spec_str(&format!("{META}{body}"), dir.path()) {
			Ok(_) => Vec::new(),
			Err(SpecError::Invalid { problems, .. }) => problems,
			Err(other) => vec![other.to_string()],
		}
	}

	/// Assert a spec body is refused with a message containing `needle`.
	fn refused(body: &str, needle: &str) {
		let found = problems(body);
		assert!(
			found.iter().any(|p| p.contains(needle)),
			"expected a problem containing {needle:?}, got {found:?}"
		);
	}

	#[test]
	fn test_a_property_checker_beside_an_expectation_both_can_hold() {
		for body in [
			"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\n",
			// Both are judged: the value must equal expect and be sorted.
			"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\nexpect = [1, 2, 3]\n",
			"[[cases]]\nname = \"x\"\ncheck = { rhai = \"result.len() == 3\" }\nexpect = [1, 2, 3]\n",
			// In call mode stdout is judged on its own, beside the checker.
			"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\nexpected_stdout = \"hi\\n\"\n",
			"[[cases]]\nname = \"x\"\nexpect = [1]\n[cases.parametrize]\ncount = 1\n[cases.parametrize.args]\na = \"int(0, 1)\"\n[cases.parametrize.oracle]\ncheck = \"sorted\"\n",
		] {
			assert_eq!(problems(body), Vec::<String>::new(), "{body}");
		}
	}

	#[test]
	fn test_a_valid_spec_has_no_problems() {
		assert!(
			problems(
				r#"
[[setup]]
id = "db"
function = "load"

[[cases]]
name = "pure"
args = [1]
expect = 2

[[cases]]
name = "uses setup"
args = ["$db", "$$literal"]
check = { rhai = "result != ()" }

[[scenarios]]
name = "account"
[[scenarios.setup]]
id = "acct"
function = "Account"
args = [100]
[[scenarios.steps]]
name = "deposit"
id = "after"
method = "deposit"
object = "acct"
args = [50]
expect = 150
[[scenarios.steps]]
name = "balance"
attribute = "balance"
object = "acct"
expect = 150
"#
			)
			.is_empty()
		);
	}

	#[test]
	fn test_unknown_fields_are_named() {
		refused(
			"[[cases]]\nname = \"x\"\nexpected = 5\n",
			"unknown field `expected`",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\ncheck = { regex = \"^a\" }\n",
			"unknown field `regex`",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\ncheck = { exec = \"v\" }\n",
			"unknown field `exec`",
		);
	}

	#[test]
	fn test_unsupported_configuration_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let cpp = load_spec_str(
			"[meta]\nname = \"t\"\nfile = \"a.cpp\"\nlanguage = \"cpp\"\ncompile = \"g++\"\ncopy_refs = false\n[[cases]]\nname = \"x\"\nscript = true\nexpected_stdout = \"\"\n",
			dir.path(),
		)
		.unwrap_err()
		.to_string();
		assert!(cpp.contains("language 'cpp' is not supported"), "{cpp}");
		assert!(cpp.contains("compile is not supported"), "{cpp}");
		assert!(cpp.contains("copy_refs was removed"), "{cpp}");

		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\ncheck = \"nonsense\"\n",
			"unknown checker 'nonsense'",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\ncheck = { rhai = \"true\", builtin = \"exact\" }\n",
			"names builtin and rhai",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\ncheck = { builtin = \"exact\", tolerance = 0.1 }\n",
			"tolerance only applies",
		);
		refused(
			"[[setup]]\nid = \"d\"\nfile = \"gen.py\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
			"setup.file is not supported",
		);
	}

	#[test]
	fn test_nothing_is_judged_against_a_default() {
		refused(
			"[[cases]]\nname = \"x\"\nargs = [1]\n",
			"declares nothing to judge",
		);
		refused(
			"[[cases]]\nname = \"x\"\ncheck = \"approx\"\n",
			"compares against expect, which is missing",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = inf\n",
			"TOML cannot hold",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\nexpect_error = \"E\"\n",
			"expect and expect_error contradict",
		);
		refused(
			"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\nexpect_error = \"E\"\n",
			"check and expect_error contradict",
		);
		refused(
			"[[cases]]\nname = \"x\"\ncheck = \"sorted\"\nexpect = [3, 1, 2]\n",
			"expect fails the sorted check itself",
		);
		refused(
			"[[cases]]\nname = \"x\"\nscript = true\ncheck = \"sorted\"\nexpected_stdout = \"1\\n\"\n",
			"the sorted checker judges a list, but a script's value is its printed text",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = [2, 1]\n[cases.parametrize]\ncount = 1\n[cases.parametrize.args]\na = \"int(0, 1)\"\n[cases.parametrize.oracle]\ncheck = \"sorted\"\n",
			"expect fails the sorted check itself",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect_error = \"E\"\n[cases.parametrize]\ncount = 1\n[cases.parametrize.args]\na = \"int(0, 1)\"\n[cases.parametrize.oracle]\ncheck = \"sorted\"\n",
			"expect_error conflicts with oracle.check",
		);
		refused(
			"[[cases]]\nname = \"x\"\n[cases.parametrize]\ncount = 2\n[cases.parametrize.args]\na = \"int(0, 1)\"\n",
			"declares nothing to judge",
		);
		refused(
			"[[cases]]\nname = \"x\"\n[cases.parametrize]\ncount = 2\n[cases.parametrize.oracle]\ncheck = \"approx\"\n",
			"oracle.check 'approx' compares against an expectation",
		);
	}

	#[test]
	fn test_rhai_is_compiled_with_strict_variables() {
		refused(
			"[[cases]]\nname = \"x\"\ncheck = { rhai = \"reslt > 0\" }\n",
			"rhai check does not compile",
		);
		assert!(
			problems("[[cases]]\nname = \"x\"\ncheck = { rhai = \"result > context.n\" }\n")
				.is_empty()
		);
	}

	#[test]
	fn test_lifecycle_rules() {
		refused(
			"[[cases]]\nname = \"x\"\nid = \"keep\"\nexpect = 1\n",
			"id only applies to scenario steps",
		);
		refused(
			"[[cases]]\nname = \"x\"\nmethod = \"m\"\nobject = \"nope\"\nexpect = 1\n",
			"object 'nope' is not an id",
		);
		refused(
			"[[setup]]\nid = \"t\"\nteacher = \"make\"\n[[cases]]\nname = \"x\"\nmethod = \"m\"\nobject = \"t\"\nexpect = 1\n",
			"object 't' is not an id an earlier student call produced",
		);
		refused(
			"[[setup]]\nid = \"s\"\nfunction = \"load\"\n[[setup]]\nid = \"t\"\nteacher = \"wrap\"\nargs = [\"$s\"]\n[[cases]]\nname = \"x\"\nexpect = 1\n",
			"a teacher call cannot take '$s'",
		);
		refused("[[scenarios]]\nname = \"s\"\n", "has no steps");
		refused(
			"[[scenarios]]\nname = \"s\"\n[[scenarios.steps]]\nname = \"p\"\n[scenarios.steps.parametrize]\ncount = 1\n[scenarios.steps.parametrize.oracle]\nrhai = \"1\"\n",
			"cannot be parametrized",
		);
		refused(
			"[[cases]]\nname = \"x\"\nexpect = 1\n[[cases]]\nname = \"x\"\nexpect = 2\n",
			"two cases have this name",
		);
	}

	#[test]
	fn test_script_cases_are_declared() {
		let dir = tempfile::tempdir().unwrap();
		let no_function = "[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\n";
		let err = load_spec_str(
			&format!(
				"{no_function}[[cases]]\nname = \"x\"\nstdin = \"1\"\nexpected_stdout = \"1\\n\"\n"
			),
			dir.path(),
		)
		.unwrap_err()
		.to_string();
		assert!(err.contains("set script = true"), "{err}");

		assert!(
			load_spec_str(
				&format!("{no_function}[[cases]]\nname = \"x\"\nscript = true\nstdin = \"1\"\nexpected_stdout = \"1\\n\"\ncheck = \"text\"\n"),
				dir.path(),
			)
			.is_ok()
		);
		refused(
			"[[cases]]\nname = \"x\"\nscript = true\nargs = [1]\nexpect = 1\n",
			"args does not apply to a script case",
		);
		refused(
			"[[setup]]\nid = \"s\"\nfunction = \"load\"\n[[cases]]\nname = \"x\"\nscript = true\nexpected_stdout = \"\"\n",
			"cannot run alongside top-level [[setup]]",
		);
	}

	#[test]
	fn test_paths_are_checked() {
		refused(
			"[[cases]]\nname = \"x\"\nexpect_files = { \"../escape.txt\" = \"\" }\n",
			"must be relative, without '..'",
		);
		let dir = tempfile::tempdir().unwrap();
		let err = load_spec_str(
			"[meta]\nname = \"t\"\nfile = \"lab.py\"\nlanguage = \"python\"\nfunction = \"f\"\nimports = [\"helpers/t.py\"]\ndata_files = [\"data/in.csv\"]\n[[cases]]\nname = \"x\"\nexpect = 1\n",
			dir.path(),
		)
		.unwrap_err()
		.to_string();
		assert!(err.contains("teacher module"), "{err}");
		assert!(
			err.contains("data file 'data/in.csv' does not exist"),
			"{err}"
		);
	}

	#[test]
	fn test_every_static_refusal_names_its_problem() {
		let table = [
			("", "has no [[cases]] and no [[scenarios]]"),
			(
				"[[cases]]\nname = \"x\"\nexpect = 1\ntimeout = 0\n",
				"timeout must be between 1 and",
			),
			(
				"[[cases]]\nname = \"x\"\nexpect = 1\ntimeout = 100000\n",
				"timeout must be between 1 and",
			),
			(
				"[[setup]]\nid = \"d\"\nfunction = \"a\"\n[[setup]]\nid = \"d\"\nfunction = \"b\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				"id 'd' is already bound",
			),
			(
				"[[setup]]\nid = \"d\"\nfunction = \"a\"\nteacher = \"b\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				"must name exactly one of function",
			),
			(
				"[[scenarios]]\nname = \"s\"\n[[scenarios.steps]]\nname = \"p\"\nscript = true\nexpected_stdout = \"\"\n",
				"cannot run the file as a script",
			),
			(
				"[[setup]]\nid = \"a\"\nfunction = \"A\"\n[[cases]]\nname = \"x\"\nattribute = \"b\"\nobject = \"a\"\nargs = [1]\nexpect = 1\n",
				"reading an attribute takes no args",
			),
			(
				"[[cases]]\nname = \"x\"\ncheck = { python = \"missing.py\" }\n",
				"checker script",
			),
			(
				"[[cases]]\nname = \"x\"\n[cases.parametrize]\ncount = 1\n[cases.parametrize.oracle]\nrhai = \"1\"\ncheck = \"sorted\"\n",
				"names exactly one of reference, rhai, check",
			),
			(
				"[[cases]]\nname = \"x\"\n[cases.parametrize]\ncount = 1\n[cases.parametrize.oracle]\ncheck = \"nope\"\n",
				"oracle.check: unknown checker 'nope'",
			),
			(
				"[[cases]]\nname = \"x\"\ncheck = { rhai = \"result != ()\" }\n[cases.parametrize]\ncount = 1\n[cases.parametrize.oracle]\ncheck = \"sorted\"\n",
				"check conflicts with oracle.check",
			),
			(
				"[[cases]]\nname = \"x\"\nexpect_error = \"E\"\n[cases.parametrize]\ncount = 1\n[cases.parametrize.oracle]\nrhai = \"1\"\n",
				"expect_error conflicts with an oracle",
			),
			(
				"[[setup]]\nid = \"a\"\nfunction = \"A\"\n[[cases]]\nname = \"x\"\nattribute = \"b\"\nobject = \"a\"\n[cases.parametrize]\ncount = 1\n[cases.parametrize.oracle]\nrhai = \"1\"\n",
				"cannot be parametrized",
			),
			(
				"[[cases]]\nname = \"x\"\nexpect = \"3.0\"\ncheck = \"approx\"\n",
				"the approx checker needs a number",
			),
			(
				"[[cases]]\nname = \"x\"\nexpect = 3\ncheck = \"set_eq\"\n",
				"the set_eq checker needs an array",
			),
			(
				"[[cases]]\nname = \"x\"\nexpect = 3\ncheck = \"text\"\n",
				"the text checker needs a string",
			),
			(
				"[[cases]]\nname = \"x\"\nargs = [inf]\nexpect = 1\n",
				"args contain a value TOML cannot pass",
			),
			(
				"[vars]\nstdout = 1\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				"'stdout' is reserved",
			),
			(
				"[[setup]]\nid = \"stdout\"\nfunction = \"a\"\n[[cases]]\nname = \"x\"\nexpect = 1\n",
				"id 'stdout' is reserved",
			),
		];
		for (body, needle) in table {
			refused(body, needle);
		}
		let long = "x".repeat(STDOUT_LIMIT + 1);
		refused(
			&format!("[[cases]]\nname = \"x\"\nexpected_stdout = \"{long}\"\n"),
			"longer than the 64 KiB",
		);
	}

	#[test]
	fn test_a_script_case_refuses_a_function_checker() {
		refused(
			"[[cases]]\nname = \"x\"\nscript = true\nexpected_stdout = \"1\"\ncheck = { function = \"chk\" }\n",
			"check = { function } (it judges a returned value",
		);
	}

	#[test]
	fn test_a_bare_relative_filename_means_the_current_directory() {
		assert_eq!(spec_dir(Path::new("t.toml")), Path::new("."));
		assert_eq!(spec_dir(Path::new("tests/t.toml")), Path::new("tests"));
	}
}
