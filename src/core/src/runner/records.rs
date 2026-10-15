//! Reading the teacher side's record stream.
//!
//! Each record is a frame: a 4-byte big-endian length, then that many bytes of JSON. A
//! frame longer than the unit's largest legal record, or more records than its plan can
//! produce, ends the stream there; what arrived before it stands. Only the teacher side
//! writes here: the student side's answers reach a record after the teacher side has
//! checked them.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::runner::executor::{
	CallObservation, CheckObservation, Export, Fatal, Phase, ProtocolError, Stopped, TeacherRuntime,
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
	Stopped(Stopped),
	Fatal(Fatal),
	Inspect {
		exports: BTreeMap<String, Export>,
		#[serde(default)]
		duplicates: BTreeMap<String, Vec<String>>,
	},
	Done,
}

/// The records of one run, in the order the teacher side wrote them.
#[derive(Debug, Default)]
pub struct Records {
	pub ready: bool,
	pub load: Option<CallObservation>,
	pub setup: Vec<CallObservation>,
	pub steps: Vec<CallObservation>,
	pub checks: BTreeMap<usize, CheckObservation>,
	pub stopped: Option<Stopped>,
	pub fatal: Option<Fatal>,
	pub inspect: Option<TeacherRuntime>,
	pub done: bool,
	/// The first record that was too long, repeated, arrived out of order, or did not parse.
	pub protocol_error: Option<ProtocolError>,
}

/// What a stream may hold: the longest frame, and how many frames.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
	pub frame: usize,
	pub count: usize,
}

/// Reads frames as they arrive.
#[derive(Debug)]
pub struct Reader {
	bounds: Bounds,
	seen: usize,
	records: Records,
}

impl Reader {
	pub fn new(bounds: Bounds) -> Self {
		Self {
			bounds,
			seen: 0,
			records: Records::default(),
		}
	}

	/// Whether a frame announcing `size` bytes may be read. One that may not breaks the
	/// stream, and nothing after it is read.
	pub fn admits(&mut self, size: usize) -> bool {
		if self.records.protocol_error.is_some() {
			return false;
		}
		if size > self.bounds.frame {
			self.records.protocol_error = Some(ProtocolError::Flooded(format!(
				"a record of {size} bytes, longer than any this unit produces ({} bytes)",
				self.bounds.frame
			)));
			return false;
		}
		true
	}

	pub fn frame(&mut self, body: &[u8]) {
		if self.records.protocol_error.is_some() {
			return;
		}
		self.seen += 1;
		if self.seen > self.bounds.count {
			self.records.protocol_error = Some(ProtocolError::Flooded(format!(
				"more than the {} records this unit can produce",
				self.bounds.count
			)));
			return;
		}
		match serde_json::from_slice::<Record>(body) {
			Ok(record) => self.records.accept(record),
			Err(e) => {
				self.records.protocol_error = Some(ProtocolError::Unreadable(format!(
					"unreadable record ({e}): {}",
					String::from_utf8_lossy(&body[..body.len().min(200)])
				)))
			}
		}
	}

	pub fn finish(self) -> Records {
		self.records
	}
}

impl Records {
	fn tampered(&mut self, message: impl Into<String>) {
		self.protocol_error = Some(ProtocolError::Tampered(message.into()));
	}

	fn accept(&mut self, record: Record) {
		if self.done {
			return self.tampered("a record arrived after 'done'");
		}
		if self.stopped.is_some() && !matches!(record, Record::Done | Record::Fatal(_)) {
			return self.tampered("a record arrived after the student side was stopped");
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
			Record::Stopped(stopped) => self.stopped = Some(stopped),
			// A teacher module is imported before 'ready'; a fatal claiming otherwise, or a
			// second fatal, is not the harness's.
			Record::Fatal(_) if self.fatal.is_some() => self.tampered("a second 'fatal'"),
			Record::Fatal(fatal) if self.ready && fatal.stage != "harness" => {
				self.tampered(format!("a {} 'fatal' after 'ready'", fatal.stage))
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
	use crate::runner::executor::{Outcome, StopReason};

	const WIDE: Bounds = Bounds {
		frame: 1 << 20,
		count: 100,
	};

	fn read(frames: &[&str], bounds: Bounds) -> Records {
		let mut reader = Reader::new(bounds);
		for frame in frames {
			if reader.admits(frame.len()) {
				reader.frame(frame.as_bytes());
			}
		}
		reader.finish()
	}

	#[test]
	fn test_records_are_read_in_order() {
		let r = read(
			&[
				r#"{"kind":"ready"}"#,
				r#"{"kind":"call","phase":"step","index":0,"target":{"requested":"f","resolved":"f"},"outcome":{"returned":{"value":3,"type":"int"}},"stdout":"hi\n","stdout_truncated":false,"elapsed_ms":1}"#,
				r#"{"kind":"check","index":0,"verdict":{"pass":true,"message":""}}"#,
				r#"{"kind":"done"}"#,
			],
			WIDE,
		);
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
	fn test_a_repeated_record_is_a_protocol_error() {
		let call = r#"{"kind":"call","phase":"step","index":0,"outcome":{"timeout":{}}}"#;
		let r = read(&[call, call], WIDE);
		assert_eq!(r.steps.len(), 1);
		assert!(matches!(
			r.protocol_error,
			Some(ProtocolError::Tampered(m)) if m.contains("step record 0")
		));
	}

	#[test]
	fn test_a_garbled_record_mid_stream_is_unreadable() {
		let r = read(
			&[r#"{"kind":"ready"}"#, "{not json", r#"{"kind":"done"}"#],
			WIDE,
		);
		assert!(r.ready && !r.done);
		assert!(matches!(
			r.protocol_error,
			Some(ProtocolError::Unreadable(_))
		));
	}

	#[test]
	fn test_a_record_past_its_bound_ends_the_stream_and_earlier_records_stand() {
		let call = r#"{"kind":"call","phase":"step","index":0,"outcome":{"timeout":{}}}"#;
		let long = format!(
			r#"{{"kind":"call","phase":"step","index":1,"outcome":{{"missing":{{"message":"{}"}}}}}}"#,
			"x".repeat(200)
		);
		let tight = Bounds {
			frame: 150,
			count: 100,
		};
		let r = read(
			&[r#"{"kind":"ready"}"#, call, &long, r#"{"kind":"done"}"#],
			tight,
		);
		assert_eq!(r.steps.len(), 1, "the record before the long one stands");
		assert!(!r.done);
		assert!(matches!(r.protocol_error, Some(ProtocolError::Flooded(_))));

		let few = Bounds {
			frame: 1 << 20,
			count: 2,
		};
		let r = read(&[r#"{"kind":"ready"}"#, call, r#"{"kind":"done"}"#], few);
		assert!(
			matches!(r.protocol_error, Some(ProtocolError::Flooded(m)) if m.contains("2 records"))
		);
	}

	#[test]
	fn test_a_fault_claimed_after_ready_is_tampering() {
		let fatal =
			r#"{"kind":"fatal","stage":"teacher_import","error":{"type":"E","message":""}}"#;
		let r = read(&[r#"{"kind":"ready"}"#, fatal], WIDE);
		assert!(r.fatal.is_none());
		assert!(matches!(r.protocol_error, Some(ProtocolError::Tampered(_))));
		let r = read(&[r#"{"kind":"done"}"#, r#"{"kind":"ready"}"#], WIDE);
		assert!(matches!(r.protocol_error, Some(ProtocolError::Tampered(_))));
		let harness = r#"{"kind":"fatal","stage":"harness","error":{"type":"E","message":""}}"#;
		assert!(
			read(&[r#"{"kind":"ready"}"#, harness], WIDE)
				.fatal
				.is_some()
		);
	}

	#[test]
	fn test_nothing_but_done_follows_a_stop() {
		let stopped =
			r#"{"kind":"stopped","phase":"step","index":0,"reason":"deadline","message":"late"}"#;
		let r = read(
			&[r#"{"kind":"ready"}"#, stopped, r#"{"kind":"done"}"#],
			WIDE,
		);
		assert_eq!(
			r.stopped.as_ref().map(|s| (s.phase, s.reason)),
			Some((Phase::Step, StopReason::Deadline))
		);
		assert!(r.done && r.protocol_error.is_none());
		let after = r#"{"kind":"call","phase":"step","index":0,"outcome":{"timeout":{}}}"#;
		let r = read(&[r#"{"kind":"ready"}"#, stopped, after], WIDE);
		assert!(matches!(r.protocol_error, Some(ProtocolError::Tampered(_))));
	}
}
