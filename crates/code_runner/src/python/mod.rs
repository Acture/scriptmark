use derivative::Derivative;
use pyo3::prelude::PyAnyMethods;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::{ffi, Python};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::ffi::CString;
use std::fmt::Debug;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use typed_builder::TypedBuilder;

const PYTHON_TRACE_CODE_TEMPLATE: &str = include_str!("python_trace_code.py");

macro_rules! create_python_trace {
	($code:expr, $filename:expr) => {{
		PYTHON_TRACE_CODE_TEMPLATE
			.replace("$code", $code)
			.replace("$filename", $filename)

	}};

	($code:expr) => {{
		PYTHON_TRACE_CODE_TEMPLATE
			.replace("$filename", "<Run From Rust>")
			.replace("$code", $code)
	}};
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize)]
#[derivative(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct PythonTraceRecord {
	pub trace_sequence: i64,
	pub line_number: i64,
	pub event: String,
	pub name: String,
	pub value: Option<Value>,
}


type RawTraceRecordType<'py> = (i64, i64, String, String, Bound<'py, PyAny>);

type RunCodeResultType<O> = Result<(O, Option<Result<Vec<PythonTraceRecord>, Box<dyn Error + Send + Sync>>>), Box<dyn Error + Send + Sync>>;

fn pyany_to_value(obj: &Bound<PyAny>) -> Result<Value, Box<dyn Error>> {
	Ok(
		if obj.is_none() {
			Value::Null
		} else if let Ok(b) = obj.extract::<bool>() {
			Value::Bool(b)
		} else if let Ok(i) = obj.extract::<i64>() {
			Value::Number(i.into())
		} else if let Ok(f) = obj.extract::<f64>() {
			serde_json::Number::from_f64(f).map(Value::Number).ok_or("f64 to json number failed")?
		} else if let Ok(s) = obj.extract::<String>() {
			Value::String(s)
		} else if obj.is_instance_of::<PyList>() {
			let list = obj.downcast::<PyList>().map_err(|e| e.to_string())?;
			let mut v = Vec::new();
			for item in list {
				v.push(pyany_to_value(&item)?);
			}
			Value::Array(v)
		} else if obj.is_instance_of::<PyDict>() {
			let dict = obj.downcast::<PyDict>().map_err(|e| e.to_string())?;
			let mut m = serde_json::Map::new();
			for (k, v) in dict {
				let key = pyany_to_value(&k)?;
				m.insert(key.to_string(), pyany_to_value(&v)?);
			}
			Value::Object(m)
		} else {
			Value::String(format!("{:?}", obj))
		}
	)
}


pub fn run_code<O>(
	code: String,
	std_in: Option<String>,
	libs_to_import: &[String],
	enable_trace: bool,
	timeout_secs: Option<usize>,
) -> RunCodeResultType<O>
where
	O: for<'a> FromPyObject<'a> + Clone + Send + Sync + 'static,
{
	let wrapped_code = CString::new(
		match enable_trace {
			true => create_python_trace!(&code, "<Run From Rust>"),
			false => code,
		}
	)?;

	let flush_code = CString::new("import sys; sys.stdout.flush(); sys.stderr.flush()")?;

	match timeout_secs {
		None => run_code_internal(&std_in, libs_to_import, enable_trace, &wrapped_code, &flush_code),
		Some(timeout_secs) => {
			let (tx, rx) = mpsc::channel();
			let libs_to_import_owned: Vec<String> = libs_to_import.to_vec();
			thread::spawn(move || {
				let res = run_code_internal(&std_in, &libs_to_import_owned, enable_trace, &wrapped_code, &flush_code).map_err(|e| e.to_string());
				let _ = tx.send(res);
			});
			match rx.recv_timeout(Duration::from_secs(timeout_secs as u64)) {
				Ok(res) => Ok(res?),
				Err(mpsc::RecvTimeoutError::Timeout) => {
					// 发出 KeyboardInterrupt
					Python::with_gil(|_| unsafe {
						ffi::PyErr_SetInterrupt();
					});
					Err("Execution timed out".to_string().into())
				}
				Err(e) => Err(format!("Channel error: {}", e).into()),
			}
		}
	}
}

fn run_code_internal<O>(std_in: &Option<String>, libs_to_import: &[String], enable_trace: bool, wrapped_code: &CString, flush_code: &CString)
						-> RunCodeResultType<O>
where
	O: for<'a> FromPyObject<'a> + Clone + Send + Sync + 'static,
{
	Python::with_gil(
		|py: Python<'_>| -> RunCodeResultType<O> {
			let globals = PyDict::new(py);
			globals.set_item("__name__", "__main__")?;
			py.run(flush_code, None, Some(&globals))?;

			let sys = py.import("sys")?;
			let io = py.import("io")?;
			let contextlib = py.import("contextlib")?;

			let original_stdin = sys.getattr("stdin")?;

			if let Some(std_in) = std_in {
				let override_input = io.call_method1("StringIO", (std_in,))?;
				sys.setattr("stdin", override_input)?;
			}

			for lib_to_import in libs_to_import {
				let lib = py.import(lib_to_import)?;
				globals.set_item(lib_to_import, lib)?;
			}

			let original_stdout = sys.getattr("stdout")?;

			let captured_output = io.call_method0("StringIO")?;
			let redirect_stdout = contextlib.getattr("redirect_stdout")?.call1((&captured_output,))?;
			let redirect_stderr = contextlib
				.getattr("redirect_stderr")?
				.call1((&captured_output,))?;
			redirect_stdout.call_method0("__enter__")?;
			redirect_stderr.call_method0("__enter__")?;
			// sys.setattr("stdout", &captured_output)?;

			if enable_trace {
				sys.call_method1("settrace", (py.None(),))?;
			}

			py.run(&wrapped_code, Some(&globals), Some(&globals))?;

			redirect_stdout.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
			redirect_stderr.call_method1("__exit__", (py.None(), py.None(), py.None()))?;

			let captured_output = captured_output
				.getattr("getvalue")?
				.call0()?
				.extract::<O>()?;

			let trace_output: Option<Result<Vec<PythonTraceRecord>, Box<dyn Error + Send + Sync>>> = match enable_trace {
				true => {
					let raw_trace_vec = globals.get_item("trace_output")?
						.ok_or("trace_output not found")?
						.extract::<Vec<RawTraceRecordType>>()?;

					Some(
						Ok(
							raw_trace_vec.iter().map(
								|(line_number, line_num, event, name, value)| {
									PythonTraceRecord {
										trace_sequence: *line_number,
										line_number: *line_num,
										event: event.clone(),
										name: name.clone(),
										value: pyany_to_value(value).ok(),
									}
								}
							).collect()
						)
					)
				}
				false => None
			};

			sys.setattr("stdin", original_stdin)?;
			sys.setattr("stdout", original_stdout)?;

			Ok((captured_output, trace_output))
			}
		)
}

pub fn run_from_file<O>(
	code_path: &Path,
	std_in: Option<String>,
	libs_to_import: &[String],
	enable_trace: bool,
	timeout_secs: Option<usize>,
) -> RunCodeResultType<O>
where
	O: for<'a> FromPyObject<'a> + Clone + Send + Sync + 'static,
{
	let code = std::fs::read_to_string(code_path)?;

	run_code::<O>(code, std_in, libs_to_import, enable_trace, timeout_secs)
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_create_python_trace() {
		let code = PYTHON_TRACE_CODE_TEMPLATE.replace("$code", "a = 1\nb = 2\nprint(a + b)");
		print!("{}", code);
	}

	#[test]
	fn test_create_python_trace_with_trace() {
		let code = create_python_trace!("a = 1\nb = 2\nprint(a + b)", "<Run From Rust>");
		print!("{}", code);
	}
	#[test]
	fn test_run_python_code() -> Result<(), Box<dyn Error>> {
		let code = r#"
import sys
t = input()
print(t)
"#;
		let (output, _) = run_code::<String>(code.to_string(), Some("test".to_string()), &[], false, None).map_err(|e| e.to_string())?;
		println!(
			"output: {:?}, trace: {:?}",
			output,
			""
		);
		assert_eq!(output, "test\n");

		Ok(())
	}

	#[test]
	fn test_run_python_code_with_trace() -> Result<(), Box<dyn Error>> {
		let code = r#"
a = list(range(0, 100))
b = a[21::2]
b.append(123)
b.extend(list(range(200, 301, 2)))
"#;
		let (output, trace) = run_code::<String>(code.to_string(), None::<_>, &[], true, None).map_err(|e| e.to_string())?;
		assert_eq!(output, "");

		let mut traces = trace.ok_or("trace not found")?.map_err(|e| e.to_string())?;
		traces.sort_by_key(|trace| trace.trace_sequence);

		assert_eq!(traces.len(), 7);

		// 按 trace_sequence 排序
		traces.sort_by_key(|trace| trace.trace_sequence);

		for trace in traces {
			println!("{:?}", trace)
		}

		Ok(())
	}

	#[test]
	fn test_run_python_code_with_timeout() -> Result<(), Box<dyn Error>> {
		let code = r#"
count = 0
while True:
	count += 1
	print(count)

"#
			;
		let res = run_code::<String>(code.to_string(), None::<_>, &[], false, Some(1));
		println!("{:?}", res);
		Ok(())


	}
}

#[derive(Debug, TypedBuilder, Eq, PartialEq, Hash)]
pub struct Message {
	#[builder(default=String::new())]
	pub title: String,
	#[builder(default=String::new())]
	pub description: String,
}
