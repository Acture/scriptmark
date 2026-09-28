//! Reading the harness's record stream.
//!
//! Each record is one line, `@@scriptmark:<nonce>@@ {json}`, and may follow whatever a
//! student wrote without a newline, so the prefix is looked for anywhere in a line.
//! Everything else on stdout is somebody else's output and is ignored.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::runner::executor::{
	CallObservation, CheckObservation, Export, Fatal, Phase, ProtocolError, TeacherRuntime,
};

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
		#[serde(default)]
		duplicates: BTreeMap<String, Vec<String>>,
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
	pub inspect: Option<TeacherRuntime>,
	pub done: bool,
	/// The first record that repeated, arrived out of order, or did not parse.
	pub protocol_error: Option<ProtocolError>,
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
			Err(e) => {
				records.protocol_error = Some(ProtocolError::Unreadable(format!(
					"unreadable record ({e}): {}",
					body.chars().take(200).collect::<String>()
				)))
			}
		}
	}
	records
}

impl Records {
	fn tampered(&mut self, message: impl Into<String>) {
		self.protocol_error = Some(ProtocolError::Tampered(message.into()));
	}

	fn accept(&mut self, record: Record) {
		if self.done {
			return self.tampered("a record arrived after 'done'");
		}
		match record {
			Record::Ready if self.ready => self.tampered("'ready' repeated"),
			Record::Ready => self.ready = true,
			Record::Call { phase, call } => {
				let (list, name) = match phase {
					Phase::Load => {
						if self.load.is_some() {
							self.tampered("the load record repeated");
						} else {
							self.load = Some(call);
						}
						return;
					}
					Phase::Setup => (&mut self.setup, "setup"),
					Phase::Step => (&mut self.steps, "step"),
				};
				if call.index != list.len() {
					let message = format!(
						"{name} record {} arrived where {} was expected",
						call.index,
						list.len()
					);
					self.tampered(message);
				} else {
					list.push(call);
				}
			}
			Record::Check { index, check } => {
				if index >= self.steps.len() || self.checks.contains_key(&index) {
					self.tampered(format!("check record {index} is out of place"));
				} else {
					self.checks.insert(index, check);
				}
			}
			// A teacher module is imported before 'ready'; a fatal claiming otherwise, or a
			// second fatal, did not come from the harness.
			Record::Fatal(_) if self.fatal.is_some() => self.tampered("a second 'fatal'"),
			Record::Fatal(fatal) if self.ready && fatal.stage == "teacher_import" => {
				self.tampered("a teacher_import 'fatal' after 'ready'")
			}
			Record::Fatal(fatal) => self.fatal = Some(fatal),
			Record::Inspect {
				exports,
				duplicates,
			} => {
				self.inspect = Some(TeacherRuntime {
					exports,
					duplicates,
				})
			}
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
		assert!(matches!(
			r.protocol_error,
			Some(ProtocolError::Tampered(m)) if m.contains("step record 0")
		));
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

	#[test]
	fn test_a_garbled_record_mid_stream_is_unreadable_not_tampering() {
		let stdout = format!(
			"{}@@scriptmark:{N}@@ {{not json\n{}",
			line(r#"{"kind":"ready"}"#),
			line(r#"{"kind":"done"}"#),
		);
		assert!(matches!(
			parse(&stdout, N).protocol_error,
			Some(ProtocolError::Unreadable(_))
		));
	}

	#[test]
	fn test_a_fault_claimed_after_ready_is_tampering() {
		let fatal =
			r#"{"kind":"fatal","stage":"teacher_import","error":{"type":"E","message":""}}"#;
		let r = parse(
			&format!("{}{}", line(r#"{"kind":"ready"}"#), line(fatal)),
			N,
		);
		assert!(r.fatal.is_none());
		assert!(matches!(r.protocol_error, Some(ProtocolError::Tampered(_))));
		let after_done = format!(
			"{}{}",
			line(r#"{"kind":"done"}"#),
			line(r#"{"kind":"ready"}"#)
		);
		assert!(matches!(
			parse(&after_done, N).protocol_error,
			Some(ProtocolError::Tampered(_))
		));
	}
}
