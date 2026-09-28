use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::grading::apply_grading;
use scriptmark::models::{AssignmentInput, StudentReport, TestSpec};
use scriptmark::runner::orchestrator::{RunOptions, run_all};
use scriptmark::runner::prepare::prepare;
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::{SpecError, load_spec as load_spec_file, load_specs_from_dir};

/// A test specification loaded from a TOML file.
#[pyclass(name = "TestSpec")]
#[derive(Clone)]
struct PyTestSpec {
	inner: TestSpec,
}

#[pymethods]
impl PyTestSpec {
	#[getter]
	fn name(&self) -> &str {
		&self.inner.meta.name
	}

	#[getter]
	fn file(&self) -> &str {
		&self.inner.meta.file
	}

	#[getter]
	fn function(&self) -> Option<&str> {
		self.inner.meta.function.as_deref()
	}

	#[getter]
	fn language(&self) -> &str {
		&self.inner.meta.language
	}

	#[getter]
	fn num_cases(&self) -> usize {
		self.inner.cases.len()
	}

	#[getter]
	fn num_scenarios(&self) -> usize {
		self.inner.scenarios.len()
	}

	fn __repr__(&self) -> String {
		format!(
			"TestSpec(name='{}', file='{}', cases={}, scenarios={})",
			self.inner.meta.name,
			self.inner.meta.file,
			self.inner.cases.len(),
			self.inner.scenarios.len()
		)
	}
}

/// Grading results for a single student.
#[pyclass(name = "StudentResult")]
#[derive(Clone)]
struct PyStudentResult {
	inner: StudentReport,
}

#[pymethods]
impl PyStudentResult {
	#[getter]
	fn student_id(&self) -> &str {
		&self.inner.student_id
	}

	#[getter]
	fn name(&self) -> Option<&str> {
		self.inner.student_name.as_deref()
	}

	#[getter]
	fn grade(&self) -> Option<f64> {
		self.inner.final_grade
	}

	#[getter]
	fn passed(&self) -> usize {
		self.inner.total_passed()
	}

	#[getter]
	fn total(&self) -> usize {
		self.inner.total_cases()
	}

	#[getter]
	fn pass_rate(&self) -> f64 {
		self.inner.pass_rate()
	}

	/// Return the full results as a JSON-serializable dict.
	fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
		let json_val = serde_json::to_value(&self.inner)
			.map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
		json_to_py(py, &json_val)
	}

	fn __repr__(&self) -> String {
		format!(
			"StudentResult(id='{}', grade={}, passed={}/{})",
			self.inner.student_id,
			self.inner
				.final_grade
				.map(|g| format!("{g:.1}"))
				.unwrap_or_else(|| "None".to_string()),
			self.inner.total_passed(),
			self.inner.total_cases(),
		)
	}
}

/// Discover student submission files in the given directories.
///
/// Returns a dict mapping student IDs to lists of file paths.
///
/// This is a lossy convenience view: a dict keyed on student id cannot represent duplicate
/// identities, files with no identifiable owner, or roster members who did not submit. Use
/// `load_input()` when any of those matter.
#[pyfunction]
fn discover(paths: Vec<String>) -> PyResult<HashMap<String, Vec<String>>> {
	let input = local_input(&paths)?;

	Ok(input
		.students
		.iter()
		.filter(|s| !s.files().is_empty())
		.map(|student| {
			let files = student
				.files()
				.iter()
				.map(|f| f.path.to_string_lossy().to_string())
				.collect();
			(student.identity.key.to_string(), files)
		})
		.collect())
}

/// Load the full unified input model as a dict.
///
/// Unlike `discover()`, nothing is dropped: every student carries an outcome
/// (`executable`, `submitted_empty`, `received_unmatched`, `not_submitted`), unattributable
/// files appear under `unmatched`, and anomalies appear under `diagnostics`.
#[pyfunction]
#[pyo3(signature = (paths, *, roster=None))]
fn load_input(py: Python<'_>, paths: Vec<String>, roster: Option<String>) -> PyResult<PyObject> {
	let roster = match roster {
		Some(path) => Some(
			scriptmark::roster::load_roster(Path::new(&path))
				.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?,
		),
		None => None,
	};
	let path_refs: Vec<&Path> = paths.iter().map(|p| Path::new(p.as_str())).collect();
	let input = load_local_input(
		&path_refs,
		LocalInputOptions {
			roster: roster.as_ref(),
			..Default::default()
		},
	)
	.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	let json_val = serde_json::to_value(&input)
		.map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
	json_to_py(py, &json_val)
}

fn local_input(paths: &[String]) -> PyResult<AssignmentInput> {
	let path_refs: Vec<&Path> = paths.iter().map(|p| Path::new(p.as_str())).collect();
	load_local_input(&path_refs, LocalInputOptions::default())
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

/// A bundle that cannot be graded is a ValueError, however it was found out; a path that
/// is not there is the OS error it always was.
fn spec_error(e: SpecError) -> PyErr {
	match e {
		SpecError::IoError(..) => pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string()),
		SpecError::NotADirectory(_) => {
			pyo3::exceptions::PyNotADirectoryError::new_err(e.to_string())
		}
		_ => pyo3::exceptions::PyValueError::new_err(e.to_string()),
	}
}

/// Load and validate a test specification from a TOML file.
#[pyfunction]
fn load_spec(path: String) -> PyResult<PyTestSpec> {
	let spec = load_spec_file(Path::new(&path)).map_err(spec_error)?;
	Ok(PyTestSpec { inner: spec })
}

/// Run tests for all students, returning a list of raw result dicts.
#[pyfunction]
#[pyo3(signature = (submissions, tests, *, timeout=10, python="python3"))]
fn run(
	py: Python<'_>,
	submissions: Vec<String>,
	tests: String,
	timeout: u64,
	python: &str,
) -> PyResult<PyObject> {
	let results = run_grading(&submissions, &tests, timeout, python)?;
	let json_val = serde_json::to_value(&results)
		.map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
	json_to_py(py, &json_val)
}

/// Grade all students: run tests + apply grading policy.
///
/// Returns a list of StudentResult objects.
#[pyfunction]
#[pyo3(signature = (submissions, tests, *, timeout=10, python="python3", policy="linear"))]
fn grade(
	submissions: Vec<String>,
	tests: String,
	timeout: u64,
	python: &str,
	policy: &str,
) -> PyResult<Vec<PyStudentResult>> {
	let mut reports = run_grading(&submissions, &tests, timeout, python)?;

	// Apply grading policy
	let grading_policy =
		scriptmark::models::GradingPolicy::Template(scriptmark::models::TemplatePolicy {
			template: policy.to_string(),
			lower: 60.0,
			upper: 100.0,
		});
	apply_grading(&mut reports, &grading_policy);

	reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));
	Ok(reports
		.into_iter()
		.map(|r| PyStudentResult { inner: r })
		.collect())
}

/// Shared logic: discover submissions, load specs, run orchestrator.
fn run_grading(
	submissions: &[String],
	tests: &str,
	timeout: u64,
	python: &str,
) -> PyResult<Vec<StudentReport>> {
	let input = local_input(submissions)?;

	let specs = load_specs_from_dir(Path::new(tests)).map_err(spec_error)?;

	let executor = Arc::new(PythonExecutor::with_python_cmd(python));
	let options = RunOptions {
		concurrency: None,
		python: executor.python_cmd().to_string(),
	};

	// Bridge sync PyO3 → async tokio
	let rt = tokio::runtime::Runtime::new()
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	rt.block_on(async {
		let bundles = prepare(specs, executor.clone(), timeout)
			.await
			.map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
		Ok(run_all(&input.students, bundles.into(), executor, &options).await)
	})
}

/// Convert serde_json::Value to a Python object.
fn json_to_py(py: Python<'_>, val: &serde_json::Value) -> PyResult<PyObject> {
	match val {
		serde_json::Value::Null => Ok(py.None()),
		serde_json::Value::Bool(b) => Ok(b.into_pyobject(py)?.to_owned().into_any().unbind()),
		serde_json::Value::Number(n) => {
			if let Some(i) = n.as_i64() {
				Ok(i.into_pyobject(py)?.into_any().unbind())
			} else if let Some(f) = n.as_f64() {
				Ok(f.into_pyobject(py)?.into_any().unbind())
			} else {
				Ok(py.None())
			}
		}
		serde_json::Value::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
		serde_json::Value::Array(arr) => {
			let items: Vec<PyObject> = arr
				.iter()
				.map(|v| json_to_py(py, v))
				.collect::<PyResult<_>>()?;
			Ok(items.into_pyobject(py)?.into_any().unbind())
		}
		serde_json::Value::Object(map) => {
			let dict = PyDict::new(py);
			for (k, v) in map {
				dict.set_item(k, json_to_py(py, v)?)?;
			}
			Ok(dict.into_any().unbind())
		}
	}
}

#[pymodule]
fn _scriptmark(m: &Bound<'_, PyModule>) -> PyResult<()> {
	m.add_class::<PyTestSpec>()?;
	m.add_class::<PyStudentResult>()?;
	m.add_function(wrap_pyfunction!(discover, m)?)?;
	m.add_function(wrap_pyfunction!(load_input, m)?)?;
	m.add_function(wrap_pyfunction!(load_spec, m)?)?;
	m.add_function(wrap_pyfunction!(run, m)?)?;
	m.add_function(wrap_pyfunction!(grade, m)?)?;
	Ok(())
}
