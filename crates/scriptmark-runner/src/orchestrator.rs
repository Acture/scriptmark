use std::collections::HashMap;
use std::sync::Arc;

use scriptmark_core::models::{
    CaseResult, FailureDetail, StudentFile, StudentReport, SubmissionSet, TestResult, TestSpec,
    TestStatus,
};
use tokio::sync::Semaphore;

use crate::python::PythonExecutor;
use crate::resolve::resolve_args;

/// Run all test specs for all students in parallel.
///
/// Concurrency is bounded by `max_concurrent` (defaults to number of CPUs).
pub async fn run_all(
    submissions: &SubmissionSet,
    specs: &[TestSpec],
    executor: &PythonExecutor,
    timeout_secs: u64,
    max_concurrent: Option<usize>,
) -> HashMap<String, StudentReport> {
    let concurrency = max_concurrent.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let mut handles = Vec::new();

    for (sid, files) in &submissions.by_student {
        let sid = sid.clone();
        let files = files.clone();
        let specs = specs.to_vec();
        let sem = semaphore.clone();
        let python_cmd = executor.python_cmd().to_string();
        let timeout = timeout_secs;

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let exec = PythonExecutor::with_python_cmd(&python_cmd);
            let report = run_student(&exec, &sid, &files, &specs, timeout).await;
            (sid, report)
        });

        handles.push(handle);
    }

    let mut results = HashMap::new();
    for handle in handles {
        if let Ok((sid, report)) = handle.await {
            results.insert(sid, report);
        }
    }

    results
}

/// Run all test specs for a single student.
async fn run_student(
    executor: &PythonExecutor,
    sid: &str,
    files: &[StudentFile],
    specs: &[TestSpec],
    timeout_secs: u64,
) -> StudentReport {
    let mut test_results = Vec::new();

    for spec in specs {
        // 1. Seed context with vars
        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        for (key, value) in &spec.vars {
            context.insert(key.clone(), value.clone());
        }

        // 2. Run setup steps
        let mut setup_failed = false;

        for step in &spec.setup {
            if setup_failed {
                break;
            }

            let value = if let Some(function_name) = &step.function {
                let resolved_args = resolve_args(&step.args, &context);
                let case = scriptmark_core::models::TestCase {
                    name: format!("setup:{}", step.id),
                    id: Some(step.id.clone()),
                    args: resolved_args,
                    expect: None,
                    expect_error: None,
                    stdin: None,
                    expected_stdout: None,
                    check: None,
                    timeout: None,
                    parametrize: None,
                };

                let setup_spec = TestSpec {
                    meta: scriptmark_core::models::TestMeta {
                        function: Some(function_name.clone()),
                        ..spec.meta.clone()
                    },
                    vars: Default::default(),
                    setup: vec![],
                    cases: vec![],
                };

                let result = executor
                    .execute_case(files, &setup_spec, &case, timeout_secs)
                    .await;

                if result.status != TestStatus::Passed && result.status != TestStatus::Failed {
                    setup_failed = true;
                    serde_json::Value::Null
                } else {
                    result
                        .actual
                        .as_ref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::Value::Null)
                }
            } else if let Some(script_path) = &step.file {
                // Run teacher script, capture JSON stdout
                match tokio::process::Command::new("python3")
                    .arg(script_path)
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        serde_json::from_str(stdout.trim()).unwrap_or(serde_json::Value::Null)
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!("Setup script '{}' failed: {}", script_path, stderr.trim());
                        setup_failed = true;
                        serde_json::Value::Null
                    }
                    Err(e) => {
                        eprintln!("Failed to run setup script '{}': {}", script_path, e);
                        setup_failed = true;
                        serde_json::Value::Null
                    }
                }
            } else {
                serde_json::Value::Null
            };

            context.insert(step.id.clone(), value);
        }

        // 3. Expand parametrized cases
        let expanded_cases = crate::expander::expand_cases(&spec.cases);

        // 4. Resolve oracles for parametrized cases
        let mut final_cases = Vec::new();
        for mut case in expanded_cases {
            // Find original parametrized case to get oracle config
            let original = spec
                .cases
                .iter()
                .find(|c| case.name.starts_with(&c.name) && c.parametrize.is_some());
            if let Some(orig) = original
                && let Some(param) = &orig.parametrize
            {
                let mut arg_names: Vec<String> = param.args.keys().cloned().collect();
                arg_names.sort();
                crate::oracle::resolve_oracle(&mut case, &param.oracle, spec, executor, &arg_names)
                    .await;
            }
            final_cases.push(case);
        }

        // 5. Run cases with resolved args
        let mut cases = Vec::new();
        for case in &final_cases {
            if setup_failed {
                cases.push(CaseResult {
                    case_name: case.name.clone(),
                    status: TestStatus::Error,
                    actual: None,
                    expected: None,
                    failure: Some(FailureDetail {
                        message: "Skipped: setup step failed".to_string(),
                        details: String::new(),
                    }),
                    elapsed_ms: Some(0),
                });
                continue;
            }

            let case_timeout = case.timeout.unwrap_or(timeout_secs);

            // Resolve $ref in args
            let resolved_case = scriptmark_core::models::TestCase {
                args: resolve_args(&case.args, &context),
                ..case.clone()
            };

            let result = executor
                .execute_case(files, spec, &resolved_case, case_timeout)
                .await;
            cases.push(result);
        }

        test_results.push(TestResult {
            spec_name: spec.meta.name.clone(),
            cases,
        });
    }

    StudentReport {
        student_id: sid.to_string(),
        student_name: None,
        test_results,
        final_grade: None,
        backend_name: Some("python".to_string()),
    }
}
