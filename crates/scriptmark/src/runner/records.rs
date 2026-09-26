//! Reading the harness's record stream.
//!
//! Each record is one line, `@@scriptmark:<nonce>@@ {json}`, and may follow whatever a
//! student wrote without a newline, so the prefix is looked for anywhere in a line.
//! Everything else on stdout is somebody else's output and is ignored.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::runner::executor::{CallObservation, CheckObservation, Export, Fatal, Phase};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Record {
	Ready,
	Call {
		phase: Phase,
		#[serde(flatten)]
		call: CallObservation,
	},
	Check {
		index: usize,
		#[serde(flatten)]
		check: CheckObservation,
	},
	Fatal(Fatal),
	Inspect {
		exports: BTreeMap<String, Export>,
	},
	Done,
}

/// The records of one run, in the order the harness wrote them.
#[derive(Debug, Default)]
pub struct Records {
	pub ready: bool,
	pub load: Option<CallObservation>,
	pub setup: Vec<CallObservation>,
	pub steps: Vec<CallObservation>,
	pub checks: BTreeMap<usize, CheckObservation>,
	pub fatal: Option<Fatal>,
	pub inspect: Option<BTreeMap<String, Export>>,
	pub done: bool,
	/// The first record that repeated, arrived out of order, or did not parse.
	pub protocol_error: Option<String>,
}

pub fn parse(stdout: &str, nonce: &str) -> Records {
	let prefix = format!("@@scriptmark:{nonce}@@ ");
	let mut records = Records::default();
	let lines: Vec<&str> = stdout.split('\n').collect();
	for (i, line) in lines.iter().enumerate() {
		let Some(at) = line.find(&prefix) else {
			continue;
		};
		if records.protocol_error.is_some() {
			break;
		}
		let body = &line[at + prefix.len()..];
		match serde_json::from_str::<Record>(body) {
			Ok(record) => records.accept(record),
			// A record cut off by a kill is the end of the stream, not a forgery.
			Err(_) if i + 1 == lines.len() => {}
			Err(e) => records.protocol_error = Some(format!("unreadable record ({e}): {body}")),
		}
	}
	records
}

impl Records {
	fn accept(&mut self, record: Record) {
		match record {
			Record::Ready => self.ready = true,
			Record::Call { phase, call } => {
				let (list, name) = match phase {
					Phase::Load => {
						if self.load.is_some() {
							self.protocol_error = Some("the load record repeated".into());
						} else {
							self.load = Some(call);
						}
						return;
					}
					Phase::Setup => (&mut self.setup, "setup"),
					Phase::Step => (&mut self.steps, "step"),
				};
				if call.index != list.len() {
					self.protocol_error = Some(format!(
						"{name} record {} arrived where {} was expected",
						call.index,
						list.len()
					));
				} else {
					list.push(call);
				}
			}
			Record::Check { index, check } => {
				if index >= self.steps.len() || self.checks.contains_key(&index) {
					self.protocol_error = Some(format!("check record {index} is out of place"));
				} else {
					self.checks.insert(index, check);
				}
			}
			Record::Fatal(fatal) => self.fatal = Some(fatal),
			Record::Inspect { exports } => self.inspect = Some(exports),
			Record::Done => self.done = true,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::runner::executor::Outcome;

	const N: &str = "abc123";

	fn line(json: &str) -> String {
		format!("\n@@scriptmark:{N}@@ {json}\n")
	}

	#[test]
	fn test_records_are_read_between_noise() {
		let stdout = format!(
			"student noise\n{}{}more noise{}{}",
			line(r#"{"kind":"ready"}"#),
			line(
				r#"{"kind":"call","phase":"step","index":0,"target":{"requested":"f","resolved":"f"},"outcome":{"returned":{"value":3,"type":"int"}},"stdout":"hi\n","stdout_truncated":false,"elapsed_ms":1}"#
			),
			line(r#"{"kind":"check","index":0,"verdict":{"pass":true,"message":""}}"#),
			line(r#"{"kind":"done"}"#),
		);
		let r = parse(&stdout, N);
		assert!(r.ready && r.done && r.protocol_error.is_none());
		assert_eq!(r.steps.len(), 1);
		assert_eq!(r.steps[0].stdout, "hi\n");
		assert!(matches!(
			r.steps[0].outcome,
			Outcome::Returned { ref value, .. } if value == 3
		));
		assert_eq!(
			r.checks[&0],
			CheckObservation::Verdict {
				pass: true,
				message: String::new()
			}
		);
	}

	#[test]
	fn test_a_record_glued_to_a_partial_student_line_still_counts() {
		let stdout = format!("x@@scriptmark:{N}@@ {{\"kind\":\"ready\"}}\n");
		assert!(parse(&stdout, N).ready);
	}

	#[test]
	fn test_a_forged_record_with_the_wrong_nonce_is_ignored() {
		let stdout = "@@scriptmark:guess@@ {\"kind\":\"done\"}\n";
		assert!(!parse(stdout, N).done);
	}

	#[test]
	fn test_a_repeated_record_is_a_protocol_error() {
		let call = r#"{"kind":"call","phase":"step","index":0,"outcome":{"timeout":{}}}"#;
		let r = parse(&format!("{}{}", line(call), line(call)), N);
		assert_eq!(r.steps.len(), 1);
		assert!(r.protocol_error.unwrap().contains("step record 0"));
	}

	#[test]
	fn test_a_stream_cut_short_keeps_what_arrived() {
		let stdout = format!(
			"{}{}@@scriptmark:{N}@@ {{\"kind\":\"ca",
			line(r#"{"kind":"ready"}"#),
			line(
				r#"{"kind":"call","phase":"step","index":0,"outcome":{"raised":{"type":"KeyError","types":["KeyError","LookupError"],"message":"'k'"}}}"#
			),
		);
		let r = parse(&stdout, N);
		assert_eq!(r.steps.len(), 1);
		assert!(!r.done);
		assert!(
			r.protocol_error.is_none(),
			"a record cut off by a kill is the end of the stream"
		);
	}
}
