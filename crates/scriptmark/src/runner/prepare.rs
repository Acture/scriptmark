//! Preparing a test bundle: everything that runs teacher code, once, before any student.
//!
//! `prepare` re-runs the static validation, so no hand-built `TestSpec` reaches execution
//! unchecked; imports the teacher modules to learn their exports; expands each template
//! into its concrete cases, records them, and resolves their oracles; validates the
//! expanded cases again; checks every name a call uses; dry-runs Rhai checks against the
//! most common wrong answer; and plans the units. `run_all` takes only
//! `Bundle`s, and the CLI and bindings build them only here: every bundle they run has
//! been through all of it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::checker::rhai_checker::RhaiChecker;
use crate::checker::{CheckInput, Checker};
use crate::models::{Check, SetupStep, TestCase, TestSpec};
use crate::runner::answers::{Contract, FrozenAnswers};
use crate::runner::executor::{
	CallPlan, Executor, InProcessCheck, ScriptRun, Subject, TeacherRuntime, UnitPlan,
};
use crate::runner::frozen::{Generation, replay_entry};
use crate::runner::generation::{DrawSeed, Generated, generate, seed_for};
use crate::runner::judge::Scored;
use crate::runner::oracle::{resolve_oracle, validate_references};
use crate::spec_loader::{MAX_TIMEOUT_SECS, RESERVED, refs, validate, validate_expanded};

/// A prepared test bundle: one spec, ready to run against any student.
#[derive(Debug, Serialize)]
pub struct Bundle {
	/// Validated, paths absolute, templates expanded, oracles resolved.
	pub spec: TestSpec,
	/// What the teacher modules export.
	pub teacher: TeacherRuntime,
	/// Each template's concrete inputs and how they were made, by template name.
	pub generated: BTreeMap<String, Generated>,
	/// Prepared expectations and their source/configuration fingerprints.
	pub answers: Option<FrozenAnswers>,
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

/// Where one spec's template inputs come from.
enum Source {
	Fresh(DrawSeed),
	/// The spec's frozen templates, by name.
	Replay(BTreeMap<String, Generated>, Option<FrozenAnswers>),
}

/// Prepare every spec, concurrently. Either all of them are ready or none is returned.
pub async fn prepare<E: Executor>(
	specs: Vec<TestSpec>,
	generation: &Generation,
	executor: Arc<E>,
	timeout_secs: u64,
) -> Result<Vec<Bundle>, PrepareErrors> {
	let refuse = |problem: &str| {
		Err(PrepareErrors(vec![PrepareError {
			name: "(the run)".into(),
			problems: vec![problem.to_string()],
		}]))
	};
	if specs.is_empty() {
		return refuse("no test specs were found: nothing would be graded");
	}
	if !(1..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
		return refuse(&format!(
			"the default timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"
		));
	}
	if let Generation::Replay(frozen) = generation
		&& frozen.format != crate::runner::frozen::FORMAT
	{
		return refuse("unsupported frozen bundle format: prepare a fresh bundle with this build");
	}

	let names: Vec<String> = specs.iter().map(|s| s.meta.name.clone()).collect();
	let mut seen = BTreeSet::new();
	if let Some(twice) = names.iter().find(|n| !seen.insert(n.as_str())) {
		return refuse(&format!(
			"two specs are named '{twice}': results and frozen inputs are kept by spec name"
		));
	}
	if let Generation::Replay(frozen) = generation
		&& let Some(extra) = frozen
			.specs
			.keys()
			.chain(frozen.answers.keys())
			.find(|s| !seen.contains(s.as_str()))
	{
		return refuse(&format!(
			"the frozen inputs have spec '{extra}', which this batch does not: replay them with the specs they were made from"
		));
	}
	let mut tasks = tokio::task::JoinSet::new();
	let mut index_of = std::collections::HashMap::new();
	for (index, spec) in specs.into_iter().enumerate() {
		let executor = executor.clone();
		let source = match generation {
			Generation::Fresh(draw) => Source::Fresh(*draw),
			Generation::Replay(frozen) => Source::Replay(
				frozen
					.specs
					.get(&spec.meta.name)
					.cloned()
					.unwrap_or_default(),
				frozen.answers.get(&spec.meta.name).cloned(),
			),
		};
		let handle =
			tasks.spawn(async move { prepare_one(spec, source, &*executor, timeout_secs).await });
		index_of.insert(handle.id(), index);
	}
	let mut done: Vec<(usize, Result<Bundle, PrepareError>)> = Vec::new();
	while let Some(joined) = tasks.join_next_with_id().await {
		match joined {
			Ok((id, result)) => done.push((index_of[&id], result)),
			Err(e) => {
				let index = index_of[&e.id()];
				done.push((
					index,
					Err(PrepareError {
						name: names[index].clone(),
						problems: vec![format!("preparing this bundle panicked: {e}")],
					}),
				));
			}
		}
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
	source: Source,
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
	let mut generated = BTreeMap::new();
	let mut arg_names: BTreeMap<String, Vec<String>> = BTreeMap::new();
	for case in &spec.cases {
		let Some(param) = &case.parametrize else {
			cases.push(case.clone());
			continue;
		};
		let inputs = param.inputs();
		let made = match &source {
			Source::Fresh(draw) => seed_for(&inputs, *draw)
				.map_err(|e| vec![e])
				.and_then(|seed| generate(&case.name, &inputs, seed)),
			Source::Replay(frozen, _) => match frozen.get(&case.name) {
				Some(entry) => replay_entry(&case.name, &inputs, entry),
				None => Err(vec![
					"the frozen inputs have no template by this name: draw new inputs instead of replaying".into(),
				]),
			},
		};
		let made = match made {
			Ok(made) => made,
			Err(errors) => {
				let at = format!("case '{}'", case.name);
				problems.extend(errors.into_iter().map(|e| format!("{at}: {e}")));
				continue;
			}
		};
		let names: Vec<String> = inputs.names().into_iter().map(String::from).collect();
		// A concrete case keeps everything its template says but `parametrize`: its
		// target, checks and timeout included.
		for concrete in &made.cases {
			let g = TestCase {
				name: concrete.name.clone(),
				args: concrete.args.clone(),
				parametrize: None,
				oracle: case.answer_source().cloned(),
				..case.clone()
			};
			arg_names.insert(g.name.clone(), names.clone());
			cases.push(g);
		}
		generated.insert(case.name.clone(), made);
	}
	if let Source::Replay(frozen, _) = &source {
		let templates: BTreeSet<&str> = spec
			.cases
			.iter()
			.filter(|c| c.parametrize.is_some())
			.map(|c| c.name.as_str())
			.collect();
		for extra in frozen.keys().filter(|t| !templates.contains(t.as_str())) {
			problems.push(format!(
				"the frozen inputs have template '{extra}', which the spec does not: replay them with the spec they were made from"
			));
		}
	}
	spec.cases = cases;
	if !problems.is_empty() {
		return Err(fail(problems));
	}
	let has_answers: bool = spec
		.cases
		.iter()
		.any(|c| c.oracle.as_ref().is_some_and(|o| o.computes()));
	let contract: Option<Contract> = has_answers
		.then(|| Contract::of(&spec, executor, timeout_secs))
		.transpose()
		.map_err(|e| fail(vec![e]))?;
	let answers: Option<FrozenAnswers> = match (&source, &contract) {
		(Source::Replay(_, Some(frozen)), Some(contract)) => {
			frozen
				.restore(contract, &mut spec)
				.map_err(|e| fail(vec![e]))?;
			Some(frozen.clone())
		}
		(Source::Replay(_, frozen), wanted) if frozen.is_some() || wanted.is_some() => {
			return Err(fail(vec!["frozen answers are missing or this spec no longer uses an oracle: prepare a fresh bundle without --replay".into()]));
		}
		(Source::Fresh(_), Some(contract)) => {
			validate_references(&spec, &arg_names, executor, timeout_secs)
				.await
				.map_err(|e| fail(vec![e]))?;
			let mut resolved: Vec<TestCase> = Vec::with_capacity(spec.cases.len());
			let total: usize = spec
				.cases
				.iter()
				.filter(|c| c.oracle.as_ref().is_some_and(|o| o.computes()))
				.count();
			let started: std::time::Instant = std::time::Instant::now();
			let mut answered: usize = 0;
			if total >= 20 {
				eprintln!("Preparing {total} oracle answers for '{}'", spec.meta.name);
			}
			for case in &spec.cases {
				let mut case: TestCase = case.clone();
				if let Some(oracle) = case.oracle.clone().filter(|o| o.computes()) {
					let names: &[String] = arg_names.get(&case.name).map_or(&[], Vec::as_slice);
					resolve_oracle(&mut case, &oracle, &spec, executor, names, timeout_secs)
						.await
						.map_err(|e| fail(vec![format!("case '{}': {e}", case.name)]))?;
					answered += 1;
					if total >= 20 && (answered.is_multiple_of(25) || answered == total) {
						let elapsed: f64 = started.elapsed().as_secs_f64();
						let remaining: f64 = elapsed / answered as f64 * (total - answered) as f64;
						eprintln!(
							"Oracle '{}': {answered}/{total}, {elapsed:.1}s elapsed, about {remaining:.1}s remaining",
							spec.meta.name
						);
					}
				}
				resolved.push(case);
			}
			// Never publish answers under a fingerprint taken before their sources changed.
			if *contract
				!= Contract::of(&spec, executor, timeout_secs).map_err(|e| fail(vec![e]))?
			{
				return Err(fail(vec![
					"oracle sources changed during preparation; prepare again".into(),
				]));
			}
			spec.cases = resolved;
			Some(FrozenAnswers::new(contract.clone(), &spec))
		}
		_ => None,
	};
	for case in &mut spec.cases {
		if let Some(name) = case.oracle.as_ref().and_then(|o| o.check.as_ref()) {
			case.check = Some(crate::models::CheckMethod::Builtin(name.clone()));
		}
		case.oracle = None;
	}
	// The expanded cases meet the static rules too — an oracle answer that cannot fit its
	// checker is refused now. Only once everything resolved: a case whose oracle failed has
	// no expectation, and would only add "nothing to judge" beside the real error.
	if problems.is_empty() {
		problems.extend(validate_expanded(&spec));
	}

	problems.extend(check_names(&spec, &teacher));
	problems.extend(dry_run_rhai(&spec));
	if !problems.is_empty() {
		return Err(fail(problems));
	}

	let units = plan_units(&spec, timeout_secs);
	Ok(Bundle {
		spec,
		teacher,
		generated,
		answers,
		units,
	})
}

/// Every name a call uses must mean exactly one thing.
fn check_names(spec: &TestSpec, teacher: &TeacherRuntime) -> Vec<String> {
	let mut problems: Vec<String> = teacher
		.duplicates
		.iter()
		.map(|(name, modules)| {
			format!(
				"'{name}' is exported by more than one teacher module ({})",
				modules.join(", ")
			)
		})
		.collect();
	if teacher.exports.contains_key(RESERVED) {
		problems.push(format!(
			"a teacher module exports '{RESERVED}', a name checkers reserve for the call's output"
		));
	}
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
					let positional = params.iter().take(2).filter(|p| !p.is_variadic()).count();
					if positional < 2 {
						problems.push(format!(
							"{at}: checker '{name}' must take (result, expected, ...)"
						));
					}
					for param in params.iter().skip(2) {
						// *args, **kwargs and defaulted parameters are filled only when named.
						let filled = param.is_variadic()
							|| param.default || param.name == RESERVED
							|| scope.contains(&param.name);
						if !filled {
							problems.push(format!(
								"{at}: checker '{name}' asks for '{}', which names nothing in scope",
								param.name
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
