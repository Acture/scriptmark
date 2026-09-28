//! Turning what a unit did into results.
//!
//! The harness only observes. Here every non-pass gets its owner, derived from three
//! things: which call the plan says was running, what kind of code that call runs, and
//! how the process ended. Nothing a student can print decides it.

use serde_json::{Value, json};

use crate::checker::builtin::{resolve_builtin, same};
use crate::checker::python_checker::PythonChecker;
use crate::checker::rhai_checker::RhaiChecker;
use crate::checker::{CheckInput, Checker};
use crate::models::{
	CaseInput, CaseResult, Cause, Check, FailureDetail, Fault, Target, TestCase, TestStatus,
};
use crate::runner::executor::{
	CallObservation, CheckObservation, Exit, Outcome, ProtocolError, UnitObservation, UnitPlan,
};

/// A scored call: the name its result is reported under, and what it expects.
#[derive(Debug, Clone, Copy)]
pub struct Scored<'a> {
	pub name: &'a str,
	pub case: &'a TestCase,
}

/// A verdict that applies to every scored call the unit could not reach.
#[derive(Debug, Clone)]
struct Blanket {
	status: TestStatus,
	fault: Fault,
	cause: Cause,
	message: String,
}

/// Judge one unit. `scored[i]` is `plan.steps[i]` — or, in script mode, the script run.
pub fn judge(
	plan: &UnitPlan,
	scored: &[Scored],
	obs: &UnitObservation,
	python_cmd: &str,
) -> Vec<CaseResult> {
	let target = |index: usize| plan.steps.get(index).map(|c| target_name(&c.target));
	if let Some(blanket) = unit_failure(plan, obs) {
		return scored
			.iter()
			.enumerate()
			.map(|(i, s)| blanket_result(s, &blanket, target(i)))
			.collect();
	}
	// The harness broke after student code ran: what it recorded before that still stands.
	let broken = stream_break(obs);

	let mut results = Vec::with_capacity(scored.len());
	// Once a call hangs or the process dies, nothing after it ran — on whoever's account.
	let mut stopped: Option<Blanket> = None;
	for (index, s) in scored.iter().enumerate() {
		if let Some(blanket) = &stopped {
			results.push(blanket_result(s, blanket, target(index)));
			continue;
		}
		let call = plan.steps.get(index);
		match obs.steps.get(index) {
			Some(record) => {
				let check = obs.checks.get(&index);
				if check.is_none()
					&& call.is_some_and(|c| c.check.is_some())
					&& matches!(record.outcome, Outcome::Returned { .. })
				{
					// The student's call finished; the teacher's checker never did.
					let blanket = broken.clone().unwrap_or_else(|| {
						in_flight(&obs.exit, Fault::Teacher, Cause::Checker, "its checker")
					});
					results.push(blanket_result(s, &blanket, target(index)));
					stopped = Some(broken.clone().unwrap_or_else(|| not_run(&blanket, s.name)));
					continue;
				}
				let timeout = call.map_or_else(
					|| plan.script.as_ref().map_or(0, |s| s.timeout),
					|c| c.timeout,
				);
				results.push(judge_call(
					s,
					timeout,
					record,
					check,
					plan.script.is_some(),
					python_cmd,
				));
			}
			None => {
				let blanket = match &broken {
					Some(broken) => broken.clone(),
					None if obs.done => Blanket {
						status: TestStatus::Error,
						fault: Fault::Environment,
						cause: Cause::Harness,
						message: "the harness finished without running this call".into(),
					},
					None => in_flight(&obs.exit, Fault::Student, Cause::Killed, "this call"),
				};
				results.push(blanket_result(s, &blanket, target(index)));
				stopped = Some(match &broken {
					Some(broken) => broken.clone(),
					None => not_run(&blanket, s.name),
				});
			}
		}
	}
	results
}

/// What stopped the unit before any scored call could be judged, if anything did.
fn unit_failure(plan: &UnitPlan, obs: &UnitObservation) -> Option<Blanket> {
	let blanket = |status, fault, cause, message: String| {
		Some(Blanket {
			status,
			fault,
			cause,
			message,
		})
	};
	if let Exit::Spawn(message) = &obs.exit {
		return blanket(
			TestStatus::Error,
			Fault::Environment,
			Cause::Spawn,
			message.clone(),
		);
	}
	if let Some(ProtocolError::Tampered(problem)) = &obs.protocol_error {
		// After `ready`, student code has had the chance to write; before it, nobody's has.
		let fault = if obs.ready {
			Fault::Student
		} else {
			Fault::Environment
		};
		return blanket(
			TestStatus::Error,
			fault,
			Cause::Protocol,
			format!("the unit's records cannot be trusted: {problem}"),
		);
	}
	if let Some(fatal) = obs.fatal.as_ref().filter(|f| f.stage == "teacher_import") {
		return blanket(
			TestStatus::Error,
			Fault::Teacher,
			Cause::TeacherImport,
			format!(
				"a teacher module failed to import: {}: {}",
				fatal.error.type_name, fatal.error.message
			),
		);
	}
	if !obs.ready
		&& let Some(broken) = stream_break(obs)
	{
		return Some(broken);
	}
	if !obs.ready {
		let (fault, cause, what) = if plan.imports.is_empty() {
			(Fault::Environment, Cause::Harness, "the harness")
		} else {
			(
				Fault::Teacher,
				Cause::TeacherImport,
				"importing the teacher modules",
			)
		};
		return blanket(
			TestStatus::Error,
			fault,
			cause,
			format!("{} {} ({})", death(&obs.exit).1, what, stderr_hint(obs)),
		);
	}
	if plan.script.is_some() {
		// A script that does not compile never ran.
		return match obs.load.as_ref().map(|l| &l.outcome) {
			Some(Outcome::Raised(e)) => blanket(
				TestStatus::Error,
				Fault::Student,
				Cause::Syntax,
				format!("the script does not compile: {}", e.message),
			),
			_ => None,
		};
	}

	match &obs.load {
		None if stream_break(obs).is_some() => return stream_break(obs),
		None => {
			return Some(in_flight(
				&obs.exit,
				Fault::Student,
				Cause::Killed,
				"the student module while it loaded",
			));
		}
		Some(load) => match &load.outcome {
			Outcome::Returned { .. } => {}
			Outcome::Timeout {} => {
				return blanket(
					TestStatus::Timeout,
					Fault::Student,
					Cause::Timeout,
					format!(
						"loading the student module took longer than {}s",
						plan.load_timeout
					),
				);
			}
			Outcome::Raised(e) => {
				let cause = if e.type_name == "SyntaxError" {
					Cause::Syntax
				} else {
					Cause::Load
				};
				return blanket(
					TestStatus::Error,
					Fault::Student,
					cause,
					format!(
						"the student module failed to load: {}: {}",
						e.type_name, e.message
					),
				);
			}
			other => {
				return blanket(
					TestStatus::Error,
					Fault::Environment,
					Cause::Harness,
					format!("unexpected load record: {other:?}"),
				);
			}
		},
	}

	for (index, call) in plan.setup.iter().enumerate() {
		let owner = owner_of(&call.target);
		let label = format!("setup '{}'", call.id.as_deref().unwrap_or("?"));
		match obs.setup.get(index) {
			Some(record) => match &record.outcome {
				Outcome::Returned { .. } => {}
				outcome => {
					return blanket(
						TestStatus::Error,
						owner,
						Cause::Setup,
						format!("not run: {label} {}", describe(outcome, call.timeout)),
					);
				}
			},
			None => {
				if let Some(broken) = stream_break(obs) {
					return Some(broken);
				}
				let killed = in_flight(&obs.exit, owner, Cause::Setup, &label);
				return blanket(
					TestStatus::Error,
					owner,
					Cause::Setup,
					format!("not run: {}", killed.message),
				);
			}
		}
	}
	None
}

/// The verdict for the call in flight when the process stopped.
fn in_flight(exit: &Exit, fault: Fault, cause: Cause, what: &str) -> Blanket {
	let (status, how) = death(exit);
	Blanket {
		// A hang or a kill in teacher code is never the student's timeout.
		status: if fault == Fault::Student {
			status
		} else {
			TestStatus::Error
		},
		fault,
		cause,
		message: format!("{how} {what}"),
	}
}

/// The verdict for scored calls after one that stopped the unit.
fn not_run(culprit: &Blanket, after: &str) -> Blanket {
	Blanket {
		status: TestStatus::Error,
		fault: culprit.fault,
		cause: Cause::NotRun,
		message: format!("not run: the unit stopped at '{after}'"),
	}
}

/// How a process that did not finish ended, as a status and a phrase.
fn death(exit: &Exit) -> (TestStatus, String) {
	match exit {
		Exit::Deadline => (
			TestStatus::Timeout,
			"the unit's deadline passed during".into(),
		),
		#[cfg(unix)]
		Exit::Signal(signal) if *signal == libc::SIGXCPU => (
			TestStatus::Timeout,
			"the CPU limit was exceeded during".into(),
		),
		Exit::Signal(signal) => (
			TestStatus::Error,
			format!("the process died (signal {signal}) during"),
		),
		Exit::Code(code) => (
			TestStatus::Error,
			format!("the process exited (code {code}) during"),
		),
		Exit::Spawn(message) => (
			TestStatus::Error,
			format!("the process never started ({message}) before"),
		),
	}
}

fn stderr_hint(obs: &UnitObservation) -> String {
	let tail = obs.stderr.trim();
	if tail.is_empty() {
		format!("{:?}", obs.exit)
	} else {
		tail.lines().last().unwrap_or(tail).to_string()
	}
}

fn owner_of(target: &Target) -> Fault {
	if target.is_student() {
		Fault::Student
	} else {
		Fault::Teacher
	}
}

fn describe(outcome: &Outcome, timeout: u64) -> String {
	match outcome {
		Outcome::Returned { value, .. } => format!("returned {value}"),
		Outcome::Raised(e) => format!("raised {}: {}", e.type_name, e.message),
		Outcome::Timeout {} => format!("timed out after {timeout}s"),
		Outcome::Unserialisable(e) => {
			format!("returned a value that cannot be represented: {}", e.message)
		}
		Outcome::Missing { message } => message.clone(),
		Outcome::Unresolved { name } => format!("needs '{name}', which was never produced"),
	}
}

/// The record stream stopped meaning anything part-way: the harness crashed, wrote a
/// record it could not have meant, or somebody flooded the channel. What was recorded
/// before that stands; nothing after it can be known.
fn stream_break(obs: &UnitObservation) -> Option<Blanket> {
	let (fault, cause, message) = match (&obs.fatal, &obs.protocol_error) {
		(_, Some(ProtocolError::Flooded(problem))) => (
			Fault::Student,
			Cause::Protocol,
			format!("the unit's output could not be read past this point: {problem}"),
		),
		(Some(fatal), _) if fatal.stage != "teacher_import" => (
			Fault::Environment,
			Cause::Harness,
			format!(
				"the harness failed: {}: {}",
				fatal.error.type_name, fatal.error.message
			),
		),
		(_, Some(ProtocolError::Unreadable(problem))) => (
			Fault::Environment,
			Cause::Harness,
			format!("the harness wrote a record it could not have meant: {problem}"),
		),
		_ => return None,
	};
	Some(Blanket {
		status: TestStatus::Error,
		fault,
		cause,
		message,
	})
}

/// The name a call asked for, as a plan spells it.
fn target_name(target: &Target) -> String {
	match target {
		Target::Function { name } | Target::Teacher { name } => name.clone(),
		Target::Method { object, name } | Target::Attribute { object, name } => {
			format!("{object}.{name}")
		}
	}
}

fn blanket_result(s: &Scored, blanket: &Blanket, target: Option<String>) -> CaseResult {
	CaseResult {
		case_name: s.name.to_string(),
		status: blanket.status,
		failure: Some(FailureDetail {
			message: blanket.message.clone(),
			details: String::new(),
		}),
		elapsed_ms: Some(0),
		fault: Some(blanket.fault),
		cause: Some(blanket.cause),
		input: Some(CaseInput {
			target,
			..input_of(s.case, None)
		}),
		..Default::default()
	}
}

fn input_of(case: &TestCase, record: Option<&CallObservation>) -> CaseInput {
	let resolved = record.and_then(|r| r.target.as_ref());
	CaseInput {
		target: resolved.map(|r| r.requested.clone()),
		resolved: resolved
			.filter(|r| r.resolved != r.requested)
			.map(|r| r.resolved.clone()),
		args: case.args.clone(),
		stdin: case.stdin.clone(),
	}
}

/// Accumulates one call's verdict: the first failed expectation decides it.
struct Verdict {
	result: CaseResult,
	judged: bool,
}

impl Verdict {
	fn fail(
		mut self,
		status: TestStatus,
		fault: Fault,
		cause: Cause,
		message: String,
	) -> CaseResult {
		self.result.status = status;
		self.result.fault = Some(fault);
		self.result.cause = Some(cause);
		self.result.failure = Some(FailureDetail {
			message,
			details: String::new(),
		});
		self.result
	}

	fn wrong(self, message: String) -> CaseResult {
		self.fail(TestStatus::Failed, Fault::Student, Cause::Wrong, message)
	}
}

fn judge_call(
	s: &Scored,
	timeout: u64,
	record: &CallObservation,
	check: Option<&CheckObservation>,
	script: bool,
	python_cmd: &str,
) -> CaseResult {
	let case = s.case;
	let mut verdict = Verdict {
		result: CaseResult {
			case_name: s.name.to_string(),
			status: TestStatus::Passed,
			expected: expected_of(case),
			elapsed_ms: Some(record.elapsed_ms),
			stdout: (!record.stdout.is_empty()).then(|| record.stdout.clone()),
			input: Some(input_of(case, Some(record))),
			..Default::default()
		},
		judged: false,
	};
	// A script is judged on what it printed, and a truncated capture cannot be judged — but
	// only once the script finished: one that timed out or crashed is judged on that.
	let output_unjudgeable = script
		&& record.stdout_truncated
		&& (case.expected_stdout.is_some() || case.check.is_some());
	// 1. The outcome, against `expect_error`.
	match &record.outcome {
		Outcome::Missing { message } => {
			return verdict.fail(
				TestStatus::Missing,
				Fault::Student,
				Cause::NoTarget,
				message.clone(),
			);
		}
		Outcome::Unresolved { name } => {
			return verdict.fail(
				TestStatus::Error,
				Fault::Student,
				Cause::Dependency,
				format!("not run: needs '{name}', which the call that makes it never produced"),
			);
		}
		Outcome::Timeout {} => {
			return verdict.fail(
				TestStatus::Timeout,
				Fault::Student,
				Cause::Timeout,
				format!("timed out after {timeout}s"),
			);
		}
		Outcome::Unserialisable(e) => {
			return verdict.fail(
				TestStatus::Error,
				Fault::Student,
				Cause::Unserialisable,
				format!("returned a value that cannot be represented: {}", e.message),
			);
		}
		Outcome::Raised(e) => {
			verdict.result.actual = Some(format!("{}: {}", e.type_name, e.message));
			match &case.expect_error {
				Some(want) if e.is_a(want) => {
					verdict.judged = true;
					// A script that exits with the expected error still owes its output.
					if output_unjudgeable {
						return verdict.wrong(
							"printed more than the 64 KiB kept, so its output cannot be compared"
								.into(),
						);
					}
					if script
						&& let Some(failed) = check_value(
							&mut verdict,
							case,
							&Value::String(record.stdout.clone()),
							case.expected_stdout.clone().map(Value::String),
							record,
							check,
							python_cmd,
						) {
						return failed;
					}
				}
				Some(want) => {
					return verdict.wrong(format!(
						"expected {want}, raised {}: {}",
						e.type_name, e.message
					));
				}
				None => {
					return verdict.fail(
						TestStatus::Error,
						Fault::Student,
						Cause::Raised,
						format!("{}: {}", e.type_name, e.message),
					);
				}
			}
		}
		Outcome::Returned { value, .. } => {
			if !script {
				verdict.result.actual = Some(value.to_string());
			}
			if let Some(want) = &case.expect_error {
				return verdict.wrong(format!("expected {want}, but the call returned {value}"));
			}
			// 2. The value check. In script mode the value is what it printed.
			if output_unjudgeable {
				return verdict.wrong(
					"printed more than the 64 KiB kept, so its output cannot be compared".into(),
				);
			}
			let (value, expected) = if script {
				verdict.result.actual = Some(record.stdout.clone());
				(
					Value::String(record.stdout.clone()),
					case.expected_stdout.clone().map(Value::String),
				)
			} else {
				(value.clone(), case.expect.clone())
			};
			if let Some(failed) = check_value(
				&mut verdict,
				case,
				&value,
				expected,
				record,
				check,
				python_cmd,
			) {
				return failed;
			}
		}
	}

	// 3. stdout, in call mode (in script mode it was the value).
	if !script && let Some(want) = &case.expected_stdout {
		verdict.judged = true;
		if record.stdout_truncated {
			return verdict
				.wrong("printed more than the 64 KiB kept, so stdout cannot be compared".into());
		}
		if &record.stdout != want {
			let mut result = verdict.wrong("stdout differs".into());
			if let Some(failure) = result.failure.as_mut() {
				failure.details = format!("expected:\n{want}\nactual:\n{}", record.stdout);
			}
			return result;
		}
	}

	// 4. Files left in the working directory.
	for (path, want) in &case.expect_files {
		verdict.judged = true;
		match record.files.get(path) {
			Some(Some(got)) if got == want => {}
			Some(Some(got)) => {
				let mut result = verdict.wrong(format!("file '{path}' differs"));
				if let Some(failure) = result.failure.as_mut() {
					failure.details = format!("expected:\n{want}\nactual:\n{got}");
				}
				return result;
			}
			_ => return verdict.wrong(format!("file '{path}' was not created")),
		}
	}

	if !verdict.judged {
		return verdict.fail(
			TestStatus::Error,
			Fault::Teacher,
			Cause::NothingToJudge,
			"nothing was judged: the case declared no expectation that applied".into(),
		);
	}
	verdict.result
}

/// Apply the case's value check. Returns the result when it decides the case.
fn check_value(
	verdict: &mut Verdict,
	case: &TestCase,
	value: &Value,
	expected: Option<Value>,
	record: &CallObservation,
	check: Option<&CheckObservation>,
	python_cmd: &str,
) -> Option<CaseResult> {
	let take = |verdict: &mut Verdict| {
		std::mem::replace(
			verdict,
			Verdict {
				result: CaseResult::default(),
				judged: true,
			},
		)
	};
	let method = case.check.as_ref().and_then(|m| m.resolve().ok());
	match method {
		Some(Check::Function(name)) => {
			verdict.judged = true;
			match check {
				Some(CheckObservation::Verdict { pass: true, .. }) => None,
				Some(CheckObservation::Verdict {
					pass: false,
					message,
				}) => Some(take(verdict).wrong(if message.is_empty() {
					format!("checker '{name}' said no")
				} else {
					message.clone()
				})),
				Some(CheckObservation::Unresolved { name: missing }) => Some(take(verdict).fail(
					TestStatus::Error,
					Fault::Student,
					Cause::Dependency,
					format!(
						"checker '{name}' needs '{missing}', which the call that makes it never produced"
					),
				)),
				Some(CheckObservation::Rejected { message }) => Some(take(verdict).fail(
					TestStatus::Failed,
					Fault::Student,
					Cause::Rejected,
					message.clone(),
				)),
				Some(CheckObservation::Error(e)) => Some(take(verdict).fail(
					TestStatus::Error,
					Fault::Teacher,
					Cause::Checker,
					format!(
						"checker '{name}' could not decide: {}: {}",
						e.type_name, e.message
					),
				)),
				None => Some(take(verdict).fail(
					TestStatus::Error,
					Fault::Environment,
					Cause::Harness,
					format!("checker '{name}' reported nothing"),
				)),
			}
		}
		Some(kind) => {
			verdict.judged = true;
			// A check that does not compare against the expectation leaves it to hold exactly.
			if !crate::checker::reads_expectation(&kind)
				&& let Some(want) = &expected
				&& !same(value, want)
			{
				return Some(take(verdict).wrong(format!("expected {want}, got {value}")));
			}
			let checker: Box<dyn Checker> = match &kind {
				Check::Builtin { name, tolerance } => {
					resolve_builtin(name, *tolerance).expect("validated at load")
				}
				Check::Rhai(expr) => Box::new(RhaiChecker::new(expr)),
				Check::Python(script) => {
					Box::new(PythonChecker::new(script).with_python_cmd(python_cmd))
				}
				Check::Function(_) => unreachable!("handled above"),
			};
			let input = CheckInput {
				result: value.clone(),
				expected: expected.unwrap_or(Value::Null),
				context: json!({ "stdout": record.stdout, "files": record.files }),
			};
			match checker.check(&input) {
				Ok(out) if out.pass => None,
				Ok(out) => Some(take(verdict).wrong(out.message)),
				Err(e) => {
					Some(take(verdict).fail(TestStatus::Error, e.fault, Cause::Checker, e.message))
				}
			}
		}
		None => match expected {
			Some(want) => {
				verdict.judged = true;
				(!same(value, &want))
					.then(|| take(verdict).wrong(format!("expected {want}, got {value}")))
			}
			None => None,
		},
	}
}

fn expected_of(case: &TestCase) -> Option<String> {
	case.expect
		.as_ref()
		.map(Value::to_string)
		.or_else(|| case.expect_error.as_ref().map(|e| format!("{e} raised")))
		.or_else(|| case.expected_stdout.clone())
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;
	use std::sync::Arc;

	use super::*;
	use crate::models::CheckMethod;
	use crate::runner::executor::{
		CallPlan, ErrorInfo, Fatal, InProcessCheck, ProtocolError, Resolved, ScriptRun, Subject,
	};

	fn plan(steps: usize) -> UnitPlan {
		UnitPlan {
			subject: Subject::Student,
			file: "lab.py".into(),
			script: None,
			imports: Vec::new(),
			vars: Arc::new(BTreeMap::new()),
			data_files: Vec::new(),
			allowed_imports: Vec::new(),
			load_timeout: 10,
			setup: Vec::new(),
			steps: (0..steps)
				.map(|_| call(Target::Function { name: "f".into() }))
				.collect(),
		}
	}

	fn call(target: Target) -> CallPlan {
		CallPlan {
			target,
			args: Vec::new(),
			stdin: None,
			timeout: 10,
			id: None,
			files: Vec::new(),
			check: None,
		}
	}

	fn record(index: usize, outcome: Outcome) -> CallObservation {
		CallObservation {
			index,
			target: Some(Resolved {
				requested: "f".into(),
				resolved: "f".into(),
			}),
			outcome,
			stdout: String::new(),
			stdout_truncated: false,
			files: BTreeMap::new(),
			elapsed_ms: 1,
		}
	}

	fn returned(value: Value) -> Outcome {
		Outcome::Returned {
			value,
			type_name: "x".into(),
		}
	}

	fn raised(ty: &str, mro: &[&str]) -> Outcome {
		Outcome::Raised(ErrorInfo {
			type_name: ty.into(),
			types: mro.iter().map(|s| s.to_string()).collect(),
			message: "boom".into(),
		})
	}

	fn finished(steps: Vec<CallObservation>) -> UnitObservation {
		UnitObservation {
			ready: true,
			load: Some(record(0, returned(Value::Null))),
			steps,
			done: true,
			exit: Exit::Code(0),
			..UnitObservation::not_started(Exit::Code(0))
		}
	}

	fn case(toml_body: &str) -> TestCase {
		toml::from_str(&format!("name = \"c\"\n{toml_body}")).unwrap()
	}

	fn one(case: &TestCase, plan: &UnitPlan, obs: &UnitObservation) -> CaseResult {
		judge(plan, &[Scored { name: "c", case }], obs, "python3").remove(0)
	}

	fn verdict(r: &CaseResult) -> (TestStatus, Option<Fault>, Option<Cause>) {
		(r.status, r.fault, r.cause)
	}

	#[test]
	fn test_values_and_exceptions() {
		let p = plan(1);
		let exact = case("expect = 2");
		assert_eq!(
			verdict(&one(
				&exact,
				&p,
				&finished(vec![record(0, returned(json!(2.0)))])
			)),
			(TestStatus::Passed, None, None),
			"2 == 2.0, as in Python"
		);
		assert_eq!(
			verdict(&one(
				&exact,
				&p,
				&finished(vec![record(0, returned(json!(3)))])
			)),
			(TestStatus::Failed, Some(Fault::Student), Some(Cause::Wrong))
		);
		assert_eq!(
			verdict(&one(
				&exact,
				&p,
				&finished(vec![record(0, raised("KeyError", &["KeyError"]))])
			)),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Raised))
		);
		let error = case("expect_error = \"ValueError\"");
		assert_eq!(
			verdict(&one(
				&error,
				&p,
				&finished(vec![record(
					0,
					raised("MyError", &["MyError", "ValueError", "Exception"])
				)])
			)),
			(TestStatus::Passed, None, None),
			"a subclass satisfies expect_error"
		);
		assert_eq!(
			verdict(&one(
				&error,
				&p,
				&finished(vec![record(0, returned(json!(1)))])
			)),
			(TestStatus::Failed, Some(Fault::Student), Some(Cause::Wrong))
		);
	}

	#[test]
	fn test_what_a_call_did_other_than_return() {
		let p = plan(1);
		let c = case("expect = 1");
		let cases = [
			(Outcome::Timeout {}, (TestStatus::Timeout, Cause::Timeout)),
			(
				Outcome::Missing {
					message: "function 'f' not found".into(),
				},
				(TestStatus::Missing, Cause::NoTarget),
			),
			(
				Outcome::Unresolved {
					name: "acct".into(),
				},
				(TestStatus::Error, Cause::Dependency),
			),
			(
				Outcome::Unserialisable(ErrorInfo {
					type_name: "Unserialisable".into(),
					types: vec![],
					message: "keys".into(),
				}),
				(TestStatus::Error, Cause::Unserialisable),
			),
		];
		for (outcome, (status, cause)) in cases {
			let r = one(&c, &p, &finished(vec![record(0, outcome)]));
			assert_eq!(verdict(&r), (status, Some(Fault::Student), Some(cause)));
		}
	}

	#[test]
	fn test_every_declared_expectation_must_hold() {
		let p = plan(1);
		let c =
			case("expect = 1\nexpected_stdout = \"hi\\n\"\nexpect_files = { \"out.txt\" = \"x\" }");
		let mut ok = record(0, returned(json!(1)));
		ok.stdout = "hi\n".into();
		ok.files.insert("out.txt".into(), Some("x".into()));
		assert_eq!(
			one(&c, &p, &finished(vec![ok.clone()])).status,
			TestStatus::Passed
		);

		let mut noisy = ok.clone();
		noisy.stdout = "bye\n".into();
		let r = one(&c, &p, &finished(vec![noisy]));
		assert_eq!(r.failure.unwrap().message, "stdout differs");

		let mut no_file = ok.clone();
		no_file.files.insert("out.txt".into(), None);
		let r = one(&c, &p, &finished(vec![no_file]));
		assert_eq!(r.failure.unwrap().message, "file 'out.txt' was not created");

		let mut truncated = ok;
		truncated.stdout_truncated = true;
		assert_eq!(
			one(&c, &p, &finished(vec![truncated])).status,
			TestStatus::Failed
		);
	}

	#[test]
	fn test_checkers_that_cannot_decide_are_the_teachers() {
		let p = plan(1);
		let rhai = case("check = { rhai = \"result.len() > 0\" }");
		assert_eq!(
			verdict(&one(
				&rhai,
				&p,
				&finished(vec![record(0, returned(Value::Null))])
			)),
			(
				TestStatus::Error,
				Some(Fault::Teacher),
				Some(Cause::Checker)
			)
		);
		let rhai = case("check = { rhai = \"result != () && result.len() > 0\" }");
		assert_eq!(
			verdict(&one(
				&rhai,
				&p,
				&finished(vec![record(0, returned(Value::Null))])
			)),
			(TestStatus::Failed, Some(Fault::Student), Some(Cause::Wrong))
		);

		let mut p = plan(1);
		p.steps[0].check = Some(InProcessCheck {
			function: "chk".into(),
			expected: None,
		});
		let c = case("check = { function = \"chk\" }");
		let with_check = |check: CheckObservation| {
			let mut obs = finished(vec![record(0, returned(json!(1)))]);
			obs.checks.insert(0, check);
			obs
		};
		let error = ErrorInfo {
			type_name: "TypeError".into(),
			types: vec![],
			message: "x".into(),
		};
		assert_eq!(
			verdict(&one(&c, &p, &with_check(CheckObservation::Error(error)))),
			(
				TestStatus::Error,
				Some(Fault::Teacher),
				Some(Cause::Checker)
			)
		);
		assert_eq!(
			verdict(&one(
				&c,
				&p,
				&with_check(CheckObservation::Rejected {
					message: "not a list".into()
				})
			)),
			(
				TestStatus::Failed,
				Some(Fault::Student),
				Some(Cause::Rejected)
			)
		);
		// A checker still running when the process stopped is the teacher's hang.
		let mut obs = finished(vec![record(0, returned(json!(1)))]);
		obs.done = false;
		obs.exit = Exit::Deadline;
		assert_eq!(
			verdict(&one(&c, &p, &obs)),
			(
				TestStatus::Error,
				Some(Fault::Teacher),
				Some(Cause::Checker)
			)
		);
	}

	#[test]
	fn test_a_case_that_judged_nothing_is_the_teachers() {
		let p = plan(1);
		let c = TestCase {
			name: "c".into(),
			..Default::default()
		};
		assert_eq!(
			verdict(&one(&c, &p, &finished(vec![record(0, returned(json!(1)))]))),
			(
				TestStatus::Error,
				Some(Fault::Teacher),
				Some(Cause::NothingToJudge)
			)
		);
	}

	#[test]
	fn test_a_death_blames_the_call_in_flight_and_stops_the_rest() {
		let p = plan(3);
		let c = case("expect = 1");
		let scored = [
			Scored {
				name: "a",
				case: &c,
			},
			Scored {
				name: "b",
				case: &c,
			},
			Scored {
				name: "c",
				case: &c,
			},
		];
		let mut obs = finished(vec![record(0, returned(json!(1)))]);
		obs.done = false;
		obs.exit = Exit::Signal(libc::SIGXCPU);
		let r = judge(&p, &scored, &obs, "python3");
		assert_eq!(r[0].status, TestStatus::Passed);
		assert_eq!(
			verdict(&r[1]),
			(
				TestStatus::Timeout,
				Some(Fault::Student),
				Some(Cause::Killed)
			)
		);
		assert_eq!(
			verdict(&r[2]),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::NotRun))
		);

		obs.exit = Exit::Signal(libc::SIGSEGV);
		let r = judge(&p, &scored, &obs, "python3");
		assert_eq!(
			verdict(&r[1]),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Killed))
		);
	}

	#[test]
	fn test_unit_failures_reach_every_case() {
		let c = case("expect = 1");
		let p = plan(1);

		let spawn = UnitObservation::not_started(Exit::Spawn("no python".into()));
		assert_eq!(
			verdict(&one(&c, &p, &spawn)),
			(
				TestStatus::Error,
				Some(Fault::Environment),
				Some(Cause::Spawn)
			)
		);

		let mut teacher = UnitObservation::not_started(Exit::Code(0));
		teacher.fatal = Some(Fatal {
			stage: "teacher_import".into(),
			error: ErrorInfo {
				type_name: "NameError".into(),
				types: vec![],
				message: "checker".into(),
			},
		});
		assert_eq!(
			verdict(&one(&c, &p, &teacher)),
			(
				TestStatus::Error,
				Some(Fault::Teacher),
				Some(Cause::TeacherImport)
			)
		);

		let mut syntax = finished(vec![]);
		syntax.load = Some(record(0, raised("SyntaxError", &["SyntaxError"])));
		assert_eq!(
			verdict(&one(&c, &p, &syntax)),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Syntax))
		);

		let mut exited = finished(vec![]);
		exited.load = Some(record(
			0,
			raised("SystemExit", &["SystemExit", "BaseException"]),
		));
		assert_eq!(
			verdict(&one(&c, &p, &exited)),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Load))
		);

		let mut tampered = finished(vec![record(0, returned(json!(1)))]);
		tampered.protocol_error = Some(ProtocolError::Tampered("step record 0 repeated".into()));
		assert_eq!(
			verdict(&one(&c, &p, &tampered)),
			(
				TestStatus::Error,
				Some(Fault::Student),
				Some(Cause::Protocol)
			)
		);
	}

	#[test]
	fn test_a_failed_setup_belongs_to_whoever_owns_the_setup_call() {
		let c = case("expect = 1");
		for (target, owner) in [
			(
				Target::Function {
					name: "load".into(),
				},
				Fault::Student,
			),
			(
				Target::Teacher {
					name: "make".into(),
				},
				Fault::Teacher,
			),
		] {
			let mut p = plan(1);
			let mut setup = call(target);
			setup.id = Some("db".into());
			p.setup = vec![setup];
			let mut obs = finished(vec![]);
			obs.setup = vec![record(0, raised("OSError", &["OSError"]))];
			let r = one(&c, &p, &obs);
			assert_eq!(
				verdict(&r),
				(TestStatus::Error, Some(owner), Some(Cause::Setup))
			);
			assert!(r.failure.unwrap().message.contains("setup 'db'"));
		}
	}

	#[test]
	fn test_a_script_is_judged_on_what_it_printed() {
		let mut p = plan(0);
		p.script = Some(ScriptRun {
			stdin: None,
			timeout: 10,
			files: Vec::new(),
		});
		let mut printed = record(0, returned(Value::Null));
		printed.stdout = "6  \n".into();
		let exact = case("script = true\nexpected_stdout = \"6\\n\"");
		assert_eq!(
			one(&exact, &p, &finished(vec![printed.clone()])).status,
			TestStatus::Failed
		);
		let text = TestCase {
			check: Some(CheckMethod::Builtin("text".into())),
			..exact
		};
		assert_eq!(
			one(&text, &p, &finished(vec![printed])).status,
			TestStatus::Passed
		);
	}

	#[test]
	fn test_a_broken_stream_keeps_what_arrived_and_blames_the_harness_for_the_rest() {
		let p = plan(2);
		let c = case("expect = 1");
		let scored = [
			Scored {
				name: "a",
				case: &c,
			},
			Scored {
				name: "b",
				case: &c,
			},
		];
		for broke in [
			|obs: &mut UnitObservation| {
				obs.fatal = Some(Fatal {
					stage: "harness".into(),
					error: ErrorInfo {
						type_name: "ValueError".into(),
						types: vec![],
						message: "x".into(),
					},
				})
			},
			|obs: &mut UnitObservation| {
				obs.protocol_error = Some(ProtocolError::Unreadable("garbled".into()))
			},
		] {
			let mut obs = finished(vec![record(0, returned(json!(1)))]);
			obs.done = false;
			broke(&mut obs);
			let r = judge(&p, &scored, &obs, "python3");
			assert_eq!(r[0].status, TestStatus::Passed, "an earlier pass stands");
			assert_eq!(
				verdict(&r[1]),
				(
					TestStatus::Error,
					Some(Fault::Environment),
					Some(Cause::Harness)
				)
			);
		}
	}

	#[test]
	fn test_a_harness_that_never_got_ready_without_teacher_code_is_the_environments() {
		let obs = UnitObservation::not_started(Exit::Code(1));
		assert_eq!(
			verdict(&one(&case("expect = 1"), &plan(1), &obs)),
			(
				TestStatus::Error,
				Some(Fault::Environment),
				Some(Cause::Harness)
			)
		);
	}

	#[test]
	fn test_a_setup_that_timed_out_is_an_error_for_its_dependents() {
		let mut p = plan(1);
		let mut setup = call(Target::Function {
			name: "load".into(),
		});
		setup.id = Some("db".into());
		p.setup = vec![setup];
		let mut obs = finished(vec![]);
		obs.setup = vec![record(0, Outcome::Timeout {})];
		assert_eq!(
			verdict(&one(&case("expect = 1"), &p, &obs)),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Setup))
		);
	}

	#[test]
	fn test_a_checker_whose_dependency_never_arrived_blames_the_producer() {
		let mut p = plan(1);
		p.steps[0].check = Some(InProcessCheck {
			function: "chk".into(),
			expected: None,
		});
		let mut obs = finished(vec![record(0, returned(json!(1)))]);
		obs.checks.insert(
			0,
			CheckObservation::Unresolved {
				name: "made".into(),
			},
		);
		assert_eq!(
			verdict(&one(&case("check = { function = \"chk\" }"), &p, &obs)),
			(
				TestStatus::Error,
				Some(Fault::Student),
				Some(Cause::Dependency)
			)
		);
	}

	fn script_plan() -> UnitPlan {
		UnitPlan {
			script: Some(ScriptRun {
				stdin: None,
				timeout: 10,
				files: Vec::new(),
			}),
			..plan(0)
		}
	}

	#[test]
	fn test_a_script_is_held_to_every_expectation() {
		let p = script_plan();
		let exits = case(
			"script = true\nexpected_stdout = \"bad input\\n\"\nexpect_error = \"SystemExit\"",
		);
		let mut exited = record(0, raised("SystemExit", &["SystemExit", "BaseException"]));
		assert_eq!(
			one(&exits, &p, &finished(vec![exited.clone()])).status,
			TestStatus::Failed,
			"it exited as expected but printed nothing"
		);
		exited.stdout = "bad input\n".into();
		assert_eq!(
			one(&exits, &p, &finished(vec![exited])).status,
			TestStatus::Passed
		);

		let mut long = record(0, returned(Value::Null));
		long.stdout = "x".into();
		long.stdout_truncated = true;
		let r = one(
			&case("script = true\nexpected_stdout = \"x\""),
			&p,
			&finished(vec![long]),
		);
		assert!(r.failure.unwrap().message.contains("64 KiB"));

		let mut syntax = finished(vec![]);
		syntax.load = Some(record(0, raised("SyntaxError", &["SyntaxError"])));
		assert_eq!(
			verdict(&one(
				&case("script = true\nexpected_stdout = \"x\""),
				&p,
				&syntax
			)),
			(TestStatus::Error, Some(Fault::Student), Some(Cause::Syntax))
		);
	}

	#[test]
	fn test_a_blanket_result_still_names_what_was_asked_for() {
		let obs = UnitObservation::not_started(Exit::Spawn("no python".into()));
		let r = one(&case("expect = 1"), &plan(1), &obs);
		assert_eq!(r.input.unwrap().target.as_deref(), Some("f"));
	}

	#[test]
	fn test_a_runaway_script_is_a_timeout_not_a_wrong_answer() {
		let mut flood = record(0, Outcome::Timeout {});
		flood.stdout = "x".repeat(10);
		flood.stdout_truncated = true;
		assert_eq!(
			verdict(&one(
				&case("script = true\nexpected_stdout = \"x\""),
				&script_plan(),
				&finished(vec![flood])
			)),
			(
				TestStatus::Timeout,
				Some(Fault::Student),
				Some(Cause::Timeout)
			)
		);
	}

	#[test]
	fn test_a_flooded_channel_keeps_what_arrived() {
		let p = plan(2);
		let c = case("expect = 1");
		let scored = [
			Scored {
				name: "a",
				case: &c,
			},
			Scored {
				name: "b",
				case: &c,
			},
		];
		let mut obs = finished(vec![record(0, returned(json!(1)))]);
		obs.done = false;
		obs.protocol_error = Some(ProtocolError::Flooded("64 MiB".into()));
		let r = judge(&p, &scored, &obs, "python3");
		assert_eq!(r[0].status, TestStatus::Passed);
		assert_eq!(
			verdict(&r[1]),
			(
				TestStatus::Error,
				Some(Fault::Student),
				Some(Cause::Protocol)
			)
		);
	}
}
