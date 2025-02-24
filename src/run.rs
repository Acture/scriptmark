use pyo3::prelude::{PyAnyMethods, PyDictMethods};
use pyo3::types::PyDict;
use pyo3::Python;
use std::ffi::CString;

pub fn run_python_code<S: AsRef<str> + std::fmt::Debug>(
	code: S,
	std_in: Option<S>,
	libs_to_import: Option<&[S]>,
) -> String {
	let c_code = CString::new(code.as_ref()).expect("Transform <Code> into CString Failed");
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
		let res = py.run(c_code.as_c_str(), Some(&globals), Some(&globals));
		match res {
			Ok(_) => output
				.getattr("getvalue")
				.expect("Failed to getvalue")
				.call0()
				.expect("Failed to getvalue")
				.extract::<String>()
				.expect("Failed to getvalue"),
			Err(e) => e.to_string(),
		}
	})
}

#[cfg(test)]
mod tests {
	use crate::run::run_python_code;

	#[test]
	fn test_run_python_code() {
		let code = r#"
import sys
t = input()
print(t)"#;
		let res = run_python_code(code, Some("test"), Some(&["math"]));
		assert_eq!(res, "test\n");
	}
}
