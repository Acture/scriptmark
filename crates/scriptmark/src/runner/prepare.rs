//! Preparing a test bundle: everything that runs teacher code, once, before any student.
//!
//! `prepare` re-runs the static validation, so no hand-built `TestSpec` reaches execution
//! unchecked; imports the teacher modules to learn their exports; expands parametrized
//! cases and resolves their oracles; checks every name a call uses; dry-runs Rhai checks
//! against the most common wrong answer; and plans the units. `run_all` takes only
//! `Bundle`s, so there is no path to execution around it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::checker::rhai_checker::RhaiChecker;
use crate::checker::{CheckInput, Checker};
use crate::models::{Check, SetupStep, TestCase, TestSpec};
use crate::runner::executor::{
	CallPlan, Executor, InProcessCheck, ScriptRun, Subject, TeacherRuntime, UnitPlan,
};
use crate::runner::expander::expand_case;
use crate::runner::judge::Scored;
use crate::runner::oracle::resolve_oracle;
use crate::spec_loader::{refs, validate};

/// A prepared test bundle: one spec, ready to run against any student.
#[derive(Debug, Serialize)]
pub struct Bundle {
	/// Validated, paths absolute, parametrized cases expanded, oracles resolved.
	pub spec: TestSpec,
	/// What the teacher modules export.
	pub teacher: TeacherRuntime,
	/// The units to run per student, derived from `spec`.
	#[serde(skip)]
	pub units: Vec<Unit>,
}

/// One unit, before a student's file is filled in.
#[derive(Debug, Clone)]
pub struct Unit {
	/// `file` is empty until a student's file is known.
	pub plan: UnitPlan,
	/// The scored calls, in plan order: the name each is reported under, and the case.
	pub scored: Vec<(String, TestCase)>,
}

impl Unit {
	pub fn scored(&self) -> Vec<Scored<'_>> {
		self.scored
			.iter()
			.map(|(name, case)| Scored { name, case })
			.collect()
	}
}

#[derive(Debug, thiserror::Error)]
#[error("test bundle '{name}' cannot be graded:\n  {}", problems.join("\n  "))]
pub struct PrepareError {
	pub name: String,
	pub problems: Vec<String>,
}

/// Every bundle that failed to prepare.
#[derive(Debug, thiserror::Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct PrepareErrors(pub Vec<PrepareError>);

/// Prepare every spec, concurrently. Either all of them are ready or none is returned.
pub async fn prepare<E: Executor>(
	specs: Vec<TestSpec>,
	executor: Arc<E>,
	timeout_secs: u64,
) -> Result<Vec<Bundle>, PrepareErrors> {
	let mut tasks = tokio::task::JoinSet::new();
	for (index, spec) in specs.into_iter().enumerate() {
		let executor = executor.clone();
		tasks.spawn(async move { (index, prepare_one(spec, &*executor, timeout_secs).await) });
	}
	let mut done = Vec::new();
	while let Some(joined) = tasks.join_next().await {
		done.push(joined.map_err(|e| {
			PrepareErrors(vec![PrepareError {
				name: "?".into(),
				problems: vec![format!("preparation panicked: {e}")],
			}])
		})?);
	}
	done.sort_by_key(|(index, _)| *index);

	let mut bundles = Vec::new();
	let mut errors = Vec::new();
	for (_, result) in done {
		match result {
			Ok(bundle) => bundles.push(bundle),
			Err(e) => errors.push(e),
		}
	}
	if errors.is_empty() {
		Ok(bundles)
	} else {
		Err(PrepareErrors(errors))
	}
}

async fn prepare_one<E: Executor>(
	mut spec: TestSpec,
	executor: &E,
	timeout_secs: u64,
) -> Result<Bundle, PrepareError> {
	let name = spec.meta.name.clone();
	let fail = |problems: Vec<String>| PrepareError {
		name: name.clone(),
		problems,
	};

	let problems = validate(&spec);
	if !problems.is_empty() {
		return Err(fail(problems));
	}
	let teacher = executor
		.inspect(&spec, timeout_secs)
		.await
		.map_err(|e| fail(vec![e]))?;

	let mut problems = Vec::new();
	let mut cases = Vec::new();
	for case in &spec.cases {
		let generated = expand_case(case);
		match &case.parametrize {
			Some(param) => {
				let arg_names: Vec<String> = param.args.keys().cloned().collect();
				for mut g in generated {
					if let Err(e) = resolve_oracle(
						&mut g,
						&param.oracle,
						&spec,
						executor,
						&arg_names,
						timeout_secs,
					)
					.await
					{
						problems.push(format!("case '{}': {e}", g.name));
					}
					cases.push(g);
				}
			}
			None => cases.extend(generated),
		}
	}
	spec.cases = cases;

	problems.extend(check_names(&spec, &teacher));
	problems.extend(dry_run_rhai(&spec));
	if !problems.is_empty() {
		return Err(fail(problems));
	}

	let units = plan_units(&spec, timeout_secs);
	Ok(Bundle {
		spec,
		teacher,
		units,
	})
}

/// Every name a call uses must mean exactly one thing.
fn check_names(spec: &TestSpec, teacher: &TeacherRuntime) -> Vec<String> {
	let mut problems = Vec::new();
	let exports: BTreeSet<&str> = teacher.exports.keys().map(String::as_str).collect();
	let mut top: BTreeSet<String> = BTreeSet::new();
	for name in spec.vars.keys() {
		if exports.contains(name.as_str()) {
			problems.push(format!(
				"'{name}' is both a [vars] entry and a teacher export"
			));
		}
		top.insert(name.clone());
	}
	top.extend(exports.iter().map(|s| s.to_string()));

	let bind = |scope: &mut BTreeSet<String>, id: &str, at: &str, problems: &mut Vec<String>| {
		if exports.contains(id) {
			problems.push(format!("{at}: id '{id}' is also a teacher export"));
		}
		scope.insert(id.to_string());
	};

	for step in &spec.setup {
		let at = format!("setup '{}'", step.id);
		problems.extend(check_setup(step, &top, teacher, &at));
		bind(&mut top, &step.id, &at, &mut problems);
	}
	for case in &spec.cases {
		problems.extend(check_call(
			case,
			&top,
			teacher,
			&format!("case '{}'", case.name),
		));
	}
	for scenario in &spec.scenarios {
		let mut scope = top.clone();
		for step in &scenario.setup {
			let at = format!("scenario '{}' setup '{}'", scenario.name, step.id);
			problems.extend(check_setup(step, &scope, teacher, &at));
			bind(&mut scope, &step.id, &at, &mut problems);
		}
		for step in &scenario.steps {
			let at = format!("scenario '{}' step '{}'", scenario.name, step.name);
			problems.extend(check_call(step, &scope, teacher, &at));
			if let Some(id) = &step.id {
				bind(&mut scope, id, &at, &mut problems);
			}
		}
	}
	problems
}

fn unknown_refs(args: &[Value], scope: &BTreeSet<String>, at: &str) -> Vec<String> {
	refs(args)
		.into_iter()
		.filter(|name| !scope.contains(name))
		.map(|name| {
			format!(
				"{at}: '${name}' names nothing in scope (write '$${name}' for a literal string)"
			)
		})
		.collect()
}

fn check_setup(
	step: &SetupStep,
	scope: &BTreeSet<String>,
	teacher: &TeacherRuntime,
	at: &str,
) -> Vec<String> {
	let mut problems = unknown_refs(&step.args, scope, at);
	if let Some(name) = &step.teacher
		&& !teacher.exports.get(name).is_some_and(|e| e.callable)
	{
		problems.push(format!(
			"{at}: teacher function '{name}' is not exported by the teacher modules"
		));
	}
	problems
}

fn check_call(
	case: &TestCase,
	scope: &BTreeSet<String>,
	teacher: &TeacherRuntime,
	at: &str,
) -> Vec<String> {
	let mut problems = unknown_refs(&case.args, scope, at);
	if let Some(Ok(Check::Function(name))) = case.check.as_ref().map(|c| c.resolve()) {
		match teacher.exports.get(&name) {
			Some(export) if export.callable => {
				if let Some(params) = &export.params {
					if params.len() < 2 {
						problems.push(format!(
							"{at}: checker '{name}' must take (result, expected, ...)"
						));
					}
					for param in params.iter().skip(2) {
						if param != "stdout" && !scope.contains(param) {
							problems.push(format!(
								"{at}: checker '{name}' asks for '{param}', which names nothing in scope"
							));
						}
					}
				}
			}
			_ => problems.push(format!(
				"{at}: checker '{name}' is not a function the teacher modules export"
			)),
		}
	}
	problems
}

/// A Rhai check must decide for the most common wrong answer — a function that returns
/// nothing — and for the case's own expectation. One that cannot is ambiguous, and that
/// is the teacher's to resolve before any student is graded.
fn dry_run_rhai(spec: &TestSpec) -> Vec<String> {
	let steps = spec.scenarios.iter().flat_map(|s| {
		s.steps
			.iter()
			.map(move |step| (format!("scenario '{}' step '{}'", s.name, step.name), step))
	});
	let cases = spec
		.cases
		.iter()
		.map(|c| (format!("case '{}'", c.name), c))
		.chain(steps);

	let mut problems = Vec::new();
	for (at, case) in cases {
		let Some(Ok(Check::Rhai(expr))) = case.check.as_ref().map(|c| c.resolve()) else {
			continue;
		};
		let expected = if case.script {
			case.expected_stdout.clone().map(Value::String)
		} else {
			case.expect.clone()
		};
		let nothing = if case.script {
			("prints nothing", Value::String(String::new()))
		} else {
			("returns None", Value::Null)
		};
		let mut trials = vec![nothing];
		if let Some(value) = &expected {
			trials.push(("returns exactly what is expected", value.clone()));
		}
		for (label, result) in trials {
			let input = CheckInput {
				result,
				expected: expected.clone().unwrap_or(Value::Null),
				context: json!({ "stdout": "", "files": {} }),
			};
			if let Err(e) = RhaiChecker::new(&expr).check(&input) {
				problems.push(format!(
					"{at}: the rhai check cannot judge a student who {label}: {} — guard it, e.g. `result != () && ...`",
					e.message
				));
			}
		}
	}
	problems
}

fn plan_units(spec: &TestSpec, timeout_secs: u64) -> Vec<Unit> {
	let base = UnitPlan {
		subject: Subject::Student,
		file: PathBuf::new(),
		script: None,
		imports: spec.meta.imports.clone(),
		vars: Arc::new(spec.vars.clone()),
		data_files: spec
			.meta
			.data_files
			.iter()
			.map(|rel| (spec.dir.join(rel), rel.clone()))
			.collect(),
		allowed_imports: spec.meta.allowed_imports.clone(),
		load_timeout: timeout_secs,
		setup: Vec::new(),
		steps: Vec::new(),
	};
	let default_function = spec.meta.function.as_deref();
	let top_setup: Vec<CallPlan> = spec
		.setup
		.iter()
		.map(|s| setup_call(s, timeout_secs))
		.collect();

	let cases = spec.cases.iter().map(|case| {
		let plan = if case.script {
			UnitPlan {
				script: Some(ScriptRun {
					stdin: case.stdin.clone(),
					timeout: case.timeout.unwrap_or(timeout_secs),
					files: case.expect_files.keys().cloned().collect(),
				}),
				..base.clone()
			}
		} else {
			UnitPlan {
				setup: top_setup.clone(),
				steps: vec![call_of(case, default_function, timeout_secs)],
				..base.clone()
			}
		};
		Unit {
			plan,
			scored: vec![(case.name.clone(), case.clone())],
		}
	});
	let scenarios = spec.scenarios.iter().map(|scenario| {
		let timeout = scenario.timeout.unwrap_or(timeout_secs);
		Unit {
			plan: UnitPlan {
				setup: top_setup
					.iter()
					.cloned()
					.chain(scenario.setup.iter().map(|s| setup_call(s, timeout)))
					.collect(),
				steps: scenario
					.steps
					.iter()
					.map(|s| call_of(s, default_function, timeout))
					.collect(),
				..base.clone()
			},
			scored: scenario
				.steps
				.iter()
				.map(|s| (scenario.step_name(s), s.clone()))
				.collect(),
		}
	});
	cases.chain(scenarios).collect()
}

fn setup_call(step: &SetupStep, default_timeout: u64) -> CallPlan {
	CallPlan {
		target: step.target(),
		args: step.args.clone(),
		stdin: None,
		timeout: step.timeout.unwrap_or(default_timeout),
		id: Some(step.id.clone()),
		files: Vec::new(),
		check: None,
	}
}

fn call_of(case: &TestCase, default_function: Option<&str>, default_timeout: u64) -> CallPlan {
	CallPlan {
		target: case
			.target(default_function)
			.expect("validate() refuses a call case with no target"),
		args: case.args.clone(),
		stdin: case.stdin.clone(),
		timeout: case.timeout.unwrap_or(default_timeout),
		id: case.id.clone(),
		files: case.expect_files.keys().cloned().collect(),
		check: match case.check.as_ref().map(|c| c.resolve()) {
			Some(Ok(Check::Function(function))) => Some(InProcessCheck {
				function,
				expected: case.expect.clone(),
			}),
			_ => None,
		},
	}
}
