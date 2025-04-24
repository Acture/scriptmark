use anyhow::anyhow;
use anyhow::Result;
use pyo3::prelude::PyAnyMethods;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::Python;
use std::collections::HashMap;
use std::ffi::CString;
use std::fmt::Debug;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use typed_builder::TypedBuilder;

macro_rules! create_python_trace {
	($code:expr) => {
		r#"import sys

trace_output = dict()
exclude_names = set(["trace_output", "trace_func", "sys", "exclude_names", "is_basic_type_or_container"])

def is_basic_type_or_container(v, depth=0, max_depth=3):
	# 防止递归太深
	if depth > max_depth:
		return False

	# 基本类型判断
	if isinstance(v, (int, float, str, bool, type(None))):
		return True

	# 容器类型判断
	if isinstance(v, (list, tuple)):
		return all(is_basic_type_or_container(x, depth + 1, max_depth) for x in v)

	if isinstance(v, dict):
		return (all(isinstance(k, (str, int)) for k in v.keys()) and  # 键必须是字符串或整数
				all(is_basic_type_or_container(val, depth + 1, max_depth) for val in v.values()))

	if isinstance(v, set):
		return all(is_basic_type_or_container(x, depth + 1, max_depth) for x in v)

	# 排除模块和其他复杂类型
	return False


def trace_func(frame, event, arg):
    try:
        if event == "line":
            if frame.f_code.co_filename == '<string>':
                lineno = frame.f_lineno
                local_vars = frame.f_locals.copy()
                filtered_vars = {str(k): v for k, v in local_vars.items() if not k.startswith('_') and k not in exclude_names and is_basic_type_or_container(v)}
                if filtered_vars:
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
pub fn run_code<R>(
	code: impl AsRef<str>,
	std_in: Option<impl AsRef<str>>,
	libs_to_import: Option<&[impl AsRef<str>]>,
) -> Result<R>
where
	R: for<'a> FromPyObject<'a> + Debug + Send + 'static,
{
	let (result, _) = run_code_internal::<R, Empty>(code, std_in, libs_to_import, false)?;
	Ok(result)
}

#[derive(Debug)]
struct Empty;

impl<'source> FromPyObject<'source> for Empty {
	fn extract_bound(_ob: &pyo3::Bound<'source, pyo3::PyAny>) -> pyo3::PyResult<Self> {
		Ok(Empty)
	}
}

pub fn run_code_with_trace<R, T>(
	code: impl AsRef<str>,
	std_in: Option<impl AsRef<str>>,
	libs_to_import: Option<&[impl AsRef<str>]>,
) -> Result<(R, HashMap<i64, HashMap<String, T>>)>
where
	R: for<'a> FromPyObject<'a> + Debug + Send + 'static,
	T: for<'a> FromPyObject<'a> + Debug + Send + 'static,
{
	let (result, trace) = run_code_internal(code, std_in, libs_to_import, true)?;
	Ok((result, trace.expect("Trace should be enabled")))
}

fn run_code_internal<R, T>(
	code: impl AsRef<str>,
	std_in: Option<impl AsRef<str>>,
	libs_to_import: Option<&[impl AsRef<str>]>,
	enable_trace: bool,
) -> Result<(R, Option<HashMap<i64, HashMap<String, T>>>)>
where
	R: for<'a> FromPyObject<'a> + Debug + Send + 'static,
	T: for<'a> FromPyObject<'a> + Debug + Send + 'static,
{
	let (tx, rx) = mpsc::channel();

	// 克隆数据用于线程
	let code_owned = code.as_ref().to_owned();
	let std_in_owned = std_in.map(|s| s.as_ref().to_owned());
	let libs_owned = libs_to_import.map(|libs| {
		libs.iter()
			.map(|s| s.as_ref().to_owned())
			.collect::<Vec<_>>()
	});

	let wrapped_code = if enable_trace {
		create_python_trace!(&code_owned) // 插入 trace 变量
	} else {
		code_owned
	};

	let c_code = CString::new(wrapped_code)
		.map_err(|e| anyhow!("Transform <Code> into CString Failed: {:?}", e))?;

	let flush_code = CString::new("import sys; sys.stdout.flush(); sys.stderr.flush()")
		.expect("Failed to transform <Code> into CString");

	thread::spawn(move || {
		let result: Result<(
			Result<R, anyhow::Error>,
			Option<Result<HashMap<i64, HashMap<String, T>>>>,
		)> = Python::with_gil(|py| {
			let globals = PyDict::new(py);
			globals
				.set_item("__name__", "__main__")
				.map_err(|e| anyhow!("Failed to set __name__ as __main__: {:?}", e))?;
			py.run(&flush_code, None, Some(&globals))
				.expect("Failed to flush Python stdout/stderr");

			let sys = py
				.import("sys")
				.map_err(|e| anyhow!("Failed to load sys: {:?}", e))?;
			let io = py
				.import("io")
				.map_err(|e| anyhow!("Failed to load io: {:?}", e))?;

			let original_stdin = sys.getattr("stdin")?;

			// 设置 stdin
			if let Some(std_in) = &std_in_owned {
				let input = io
					.call_method1("StringIO", (std_in,))
					.map_err(|e| anyhow!("Failed to prepare input: {:?}", e))?;
				sys.setattr("stdin", &input).expect("Failed to set stdin");
			}

			// 导入库
			if let Some(lib_names) = &libs_owned {
				for lib_name in lib_names {
					let lib = py
						.import(lib_name)
						.map_err(|e| anyhow!("Failed to load lib: {:?}", e))?;
					globals
						.set_item(lib_name.as_str(), lib)
						.map_err(|e| anyhow!("Failed to set lib: {:?}", e))?;
				}
			}

			let original_stdout = sys.getattr("stdout")?;

			let output = io
				.call_method0("StringIO")
				.map_err(|e| anyhow!("Failed to prepare output: {:?}", e))?;
			sys.setattr("stdout", &output)
				.map_err(|e| anyhow!("Failed to set stdout: {:?}", e))?;

			if enable_trace {
				sys.call_method1("settrace", (py.None(),))
					.map_err(|e| anyhow!("Failed to settrace: {:?}", e))?;
			}

			py.run(c_code.as_c_str(), Some(&globals), Some(&globals))
				.map_err(|e| anyhow!("PyRun Error: {:?}", e))?;

			let output = output
				.getattr("getvalue")
				.expect("Failed to getvalue")
				.call0()
				.expect("Failed to getvalue")
				.extract::<R>()
				.map_err(|e| anyhow!("Failed to extract output: {:?}", e));

			// 获取 trace 输出
			let trace_output = if enable_trace {
				match globals.get_item("trace_output") {
					Ok(Some(raw_trace_obj)) => {
						let parsed = raw_trace_obj
							.extract::<HashMap<i64, HashMap<String, PyObject>>>()
							.ok() // 提取 `PyObject` 失败时返回 `None`
							.map(|pyobj_trace_obj| {
								pyobj_trace_obj
									.into_iter()
									.filter_map(|(k, v)| {
										let parsed_map = v
											.into_iter()
											.filter_map(|(k, v)| {
												v.extract::<T>(py).ok().map(|v| (k, v))
											}) // 过滤无效的转换
											.collect::<HashMap<String, T>>();

										if parsed_map.is_empty() {
											None
										} else {
											Some((k, parsed_map))
										}
									})
									.collect::<HashMap<i64, HashMap<String, T>>>()
							})
							.ok_or(anyhow!(
								"Failed to extract trace_output: {:?}",
								raw_trace_obj
							))?;
						Some(Ok(parsed))
					}
					_ => Some(Err(anyhow!("Failed to get trace_output"))),
				}
			} else {
				None
			};

			// 恢复 stdin
			sys.setattr("stdin", original_stdin)
				.expect("Failed to restore stdin");
			sys.setattr("stdout", original_stdout)
				.expect("Failed to restore stdout");

			Ok((output, trace_output))
		});

		tx.send(result).unwrap();
	});

	// 处理 `recv()`
	match rx.recv() {
		Ok(Ok((output, trace))) => Ok((
			output.map_err(|e| anyhow!(e))?,
			match trace {
				Some(trace) => Some(trace.map_err(|e| anyhow!(e))?),
				None => None,
			},
		)),
		Ok(Err(e)) => Err(anyhow!(e)),
		Err(e) => Err(anyhow!(e)),
	}
}

pub fn run_from_file<R>(
	code_path: impl AsRef<Path>,
	std_in: Option<impl AsRef<str>>,
	libs_to_import: Option<&[impl AsRef<str>]>,
) -> Result<R>
where
	R: for<'a> FromPyObject<'a> + Debug + Send + 'static,
{
	let code = std::fs::read_to_string(code_path.as_ref())
		.map_err(|e| anyhow!("Failed to read file: {:?}", e))?;

	let (res, _) = run_code_internal::<R, Empty>(code, std_in, libs_to_import, false)?;
	Ok(res)
}

pub fn run_from_file_with_trace<R, T>(
	code_path: impl AsRef<Path>,
	std_in: Option<impl AsRef<str>>,
	libs_to_import: Option<&[impl AsRef<str>]>,
) -> Result<(R, HashMap<i64, HashMap<String, T>>)>
where
	R: for<'a> FromPyObject<'a> + Debug + Send + 'static,
	T: for<'a> FromPyObject<'a> + Debug + Send + 'static,
{
	let code = std::fs::read_to_string(code_path.as_ref())
		.map_err(|e| anyhow!("Failed to read file: {:?}", e))?;

	let (res, trace) = run_code_internal(code, std_in, libs_to_import, true)?;
	Ok((res, trace.expect("Trace should be enabled")))
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
		let res: Result<String, anyhow::Error> = run_code(code, Some("test"), Some(&["math"]));
		match res {
			Ok(output) => assert_eq!(output, "test\n"),
			Err(e) => panic!("Failed to run code: {:?}", e),
		}
	}

	#[test]
	fn test_run_python_code_with_trace() {
		let code = r#"
a = list(range(0, 100))
b = a[21::2]
b.append(123)
b.extend(list(range(200, 301, 2)))
"#;
		let res = run_code_with_trace::<String, Vec<i64>>(code, None::<String>, None::<&[String]>);
		match res {
			Ok((output, trace)) => {
				assert_eq!(output, "");
				assert_eq!(trace.len(), 3);
				println!("{:?}", trace);
			}
			Err(e) => panic!("Failed to run code: {:?}", e),
		}
	}
}

#[derive(Debug, TypedBuilder, Eq, PartialEq, Hash)]
pub struct Message {
	#[builder(default=String::new())]
	pub title: String,
	#[builder(default=String::new())]
	pub description: String,
}
