use std::collections::HashMap;
use std::path::Path;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use scriptmark_core::discovery::discover_submissions;
use scriptmark_core::grading::apply_grading;
use scriptmark_core::models::{StudentReport, TestSpec};
use scriptmark_core::spec_loader::load_specs_from_dir;
use scriptmark_runner::orchestrator::run_all;
use scriptmark_runner::python::PythonExecutor;

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

	fn __repr__(&self) -> String {
		format!(
			"TestSpec(name='{}', file='{}', cases={})",
			self.inner.meta.name,
			self.inner.meta.file,
			self.inner.cases.len()
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
#[pyfunction]
fn discover(paths: Vec<String>) -> PyResult<HashMap<String, Vec<String>>> {
	let path_refs: Vec<&Path> = paths.iter().map(|p| Path::new(p.as_str())).collect();
	let subs = discover_submissions(&path_refs, None)
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	Ok(subs
		.by_student
		.into_iter()
		.map(|(sid, files)| {
			let paths = files
				.into_iter()
				.map(|f| f.path.to_string_lossy().to_string())
				.collect();
			(sid, paths)
		})
		.collect())
}

/// Load a test specification from a TOML file.
#[pyfunction]
fn load_spec(path: String) -> PyResult<PyTestSpec> {
	let content = std::fs::read_to_string(&path)
		.map_err(|e| pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string()))?;
	let spec: TestSpec = toml::from_str(&content)
		.map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
	Ok(PyTestSpec { inner: spec })
}

/// Run tests for all students, returning raw results as dicts.
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
	let results = run_grading(&submissions, &tests, timeout, python)?;

	// Apply grading policy
	let grading_policy =
		scriptmark_core::models::GradingPolicy::Template(scriptmark_core::models::TemplatePolicy {
			template: policy.to_string(),
			lower: 60.0,
			upper: 100.0,
		});
	let mut reports: Vec<StudentReport> = results.into_values().collect();
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
) -> PyResult<HashMap<String, StudentReport>> {
	let path_refs: Vec<&Path> = submissions.iter().map(|p| Path::new(p.as_str())).collect();
	let subs = discover_submissions(&path_refs, None)
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	let specs = load_specs_from_dir(Path::new(tests))
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	let executor = PythonExecutor::with_python_cmd(python);

	// Bridge sync PyO3 → async tokio
	let rt = tokio::runtime::Runtime::new()
		.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

	let results = rt.block_on(run_all(&subs, &specs, &executor, timeout, None));
	Ok(results)
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
	m.add_function(wrap_pyfunction!(load_spec, m)?)?;
	m.add_function(wrap_pyfunction!(run, m)?)?;
	m.add_function(wrap_pyfunction!(grade, m)?)?;
	Ok(())
}
