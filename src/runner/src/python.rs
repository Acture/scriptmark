use std::sync::mpsc;
use std::fmt::Debug;
use std::thread;
use pyo3::prelude::PyAnyMethods;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::Python;
use std::collections::HashMap;
use std::ffi::CString;
use typed_builder::TypedBuilder;

pub fn run_code<S: AsRef<str> + Debug>(
	code: S,
	std_in: Option<S>,
	libs_to_import: Option<&[S]>,
) -> String {
	let (tx, rx) = mpsc::channel();

	// 克隆数据用于线程
	let code_owned = code.as_ref().to_owned();
	let std_in_owned = std_in.map(|s| s.as_ref().to_owned());
	let libs_owned = libs_to_import.map(|libs|
		libs.into_iter()
			.map(|s| s.as_ref().to_owned())
			.collect::<Vec<_>>()
	);


	let c_code = CString::new(code_owned).expect("Transform <Code> into CString Failed");
	thread::spawn(move ||
		{
			let result = Python::with_gil(|py| {
				let globals = PyDict::new(py);
				globals
					.set_item("__name__", "__main__")
					.expect("Failed to set __name__ as __main__");
				let sys = py.import("sys").expect("Failed to load sys");
				let io = py.import("io").expect("Failed to load io");
				if let Some(std_in_owned) = std_in_owned {
					let args = (std_in_owned,);
					let input = io
						.call_method1("StringIO", args)
						.expect("Failed to prepare input");
					sys.setattr("stdin", &input).expect("Failed to set stdin");
				}

				if let Some(lib_names) = libs_owned {
					for lib_name in lib_names {
						let lib = py.import(&lib_name).expect("Failed to load lib");
						globals
							.set_item(
								lib.getattr("__name__").expect(&format!(
									"Failed to get __name__ for lib {:?}",
									&lib_name
								)),
								lib,
							)
							.expect("Failed to set lib");
					}
				}

				let output = io
					.call_method0("StringIO")
					.expect("Failed to prepare output");
				sys.setattr("stdout", &output).expect("Failed to set io");
				let res = py.run(c_code.as_c_str(), Some(&globals), Some(&globals));
				match res {
					Ok(_) => output
						.getattr("getvalue")
						.expect("Failed to getvalue")
						.call0()
						.expect("Failed to getvalue")
						.extract::<String>()
						.expect("Failed to getvalue"),
					Err(e) => panic!("Failed to run code: {:?}", e),
				}
			});
			tx.send(result).unwrap();

		}
	);
	rx.recv().unwrap()
}

macro_rules! create_python_trace {
	($code:expr) => {
		r#"import sys

trace_output = {}
exclude_names = {"trace_output", "trace_func", "sys", "exclude_names"}

def trace_func(frame, event, arg):
    try:
        if event == "line":
            if frame.f_code.co_filename == '<string>':
                lineno = frame.f_lineno
                local_vars = frame.f_locals.copy()
                filtered_vars = {k: str(v) for k, v in local_vars.items() if not k.startswith('_') and k not in exclude_names}
                trace_output[lineno] = filtered_vars
    except Exception as e:
        trace_output.append(f"Trace error: {str(e)}")
    return trace_func


sys.settrace(trace_func)
try:
    exec('''"#.to_string() + $code + r#"''')
finally:
    sys.settrace(None)
"#
	};
}

pub fn run_code_with_trace<S: AsRef<str> + Debug + Send + Sync + 'static>(
	code: S,
	std_in: Option<S>,
	libs_to_import: Option<&[S]>,
) -> (String, HashMap<i64, HashMap<String, String>>) {
	let wrapped_code = create_python_trace!(code.as_ref());
	let c_code = CString::new(wrapped_code).expect("Transform <Code> into CString Failed");
	Python::with_gil(|py| {
		let globals = PyDict::new(py);
		globals
			.set_item("__name__", "__main__")
			.expect("Failed to set __name__ as __main__");
		let sys = py.import("sys").expect("Failed to load sys");
		let io = py.import("io").expect("Failed to load io");

		if let Some(std_in) = std_in {
			let input = io
				.call_method1("StringIO", (std_in.as_ref(),))
				.expect("Failed to prepare input");
			sys.setattr("stdin", &input).expect("Failed to set stdin");
		}

		// 创建跟踪输出列表

		if let Some(lib_names) = libs_to_import {
			for lib_name in lib_names {
				let lib = py.import(lib_name.as_ref()).expect("Failed to load lib");
				globals
					.set_item(
						lib.getattr("__name__").expect(&format!(
							"Failed to get __name__ for lib {:?}",
							lib_name.as_ref()
						)),
						lib,
					)
					.expect("Failed to set lib");
			}
		}

		let output = io
			.call_method0("StringIO")
			.expect("Failed to prepare output");
		sys.setattr("stdout", &output).expect("Failed to set io");
		sys.call_method1("settrace", (py.None(),))
			.expect("Failed to settrace");
		let res = py.run(c_code.as_c_str(), Some(&globals), Some(&globals));

		let output = match res {
			Ok(_) => output
				.getattr("getvalue")
				.expect("Failed to getvalue")
				.call0()
				.expect("Failed to getvalue")
				.extract::<String>()
				.expect("Failed to getvalue"),
			Err(e) => e.to_string(),
		};
		println!("{}", output);
		// 获取变量跟踪日志
		let trace_output = match globals.get_item("trace_output") {
			Ok(trace_list) => trace_list
				.expect("Failed to get trace_list")
				.extract::<HashMap<i64, HashMap<String, String>>>()
				.expect("Failed to extact trace_list"),
			Err(e) => {
				println!("Failed to get trace_output: {:?}", e);
				HashMap::new()
			}
		};

		(output, trace_output)
	})
}

#[cfg(test)]
mod tests {
	use crate::python::{run_code, run_code_with_trace};

	#[test]
	fn test_run_python_code() {
		let code = r#"
import sys
t = input()
print(t)"#;
		let res = run_code(code, Some("test"), Some(&["math"]));
		assert_eq!(res, "test\n");
	}

	#[test]
	fn test_run_python_code_with_trace() {
		let code = r#"
a = list(range(0, 100))
b = a[21::2]
b.append(123)
b.extend(list(range(200, 301, 2)))
"#;
		let (res, trace) = run_code_with_trace(code, None, None);
		println!("{}", res);
		println!("{:?}", trace);
	}
}

#[derive(Debug, TypedBuilder, Eq, PartialEq, Hash)]
pub struct Message {
	#[builder(default=String::new())]
	pub title: String,
	#[builder(default=String::new())]
	pub description: String,
}
