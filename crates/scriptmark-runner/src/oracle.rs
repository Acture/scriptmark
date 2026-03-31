use std::path::Path;

use scriptmark_core::models::spec::Oracle;
use scriptmark_core::models::{StudentFile, TestCase, TestSpec};

use crate::python::PythonExecutor;

/// Resolve the expected value for a parametrized case using the oracle.
pub async fn resolve_oracle(
    case: &mut TestCase,
    oracle: &Oracle,
    spec: &TestSpec,
    executor: &PythonExecutor,
    arg_names: &[String],
) {
    if let Some(ref_path) = &oracle.reference {
        // Run teacher's reference implementation with same function + args
        let ref_file = StudentFile {
            path: Path::new(ref_path).to_path_buf(),
            language: "python".to_string(),
        };
        let ref_spec = TestSpec {
            meta: spec.meta.clone(),
            vars: Default::default(),
            setup: vec![],
            cases: vec![],
            lint: None,
        };
        let result = executor
            .execute_case(&[ref_file], &ref_spec, case, 10)
            .await;
        if let Some(actual) = &result.actual {
            case.expect = serde_json::from_str(actual).ok();
        }
    } else if let Some(rhai_expr) = &oracle.rhai {
        // Evaluate Rhai expression with arg names as variables
        let engine = rhai::Engine::new();
        let mut scope = rhai::Scope::new();
        for (i, name) in arg_names.iter().enumerate() {
            if let Some(val) = case.args.get(i) {
                scope.push_dynamic(
                    name.as_str(),
                    crate::checker::rhai_checker::json_to_dynamic(val),
                );
            }
        }
        if let Ok(result) = engine.eval_with_scope::<rhai::Dynamic>(&mut scope, rhai_expr) {
            case.expect = Some(dynamic_to_json(&result));
        }
    } else if let Some(check_name) = &oracle.check {
        // Just set the checker — no expected value needed
        case.check = Some(scriptmark_core::models::CheckMethod::Builtin(
            check_name.clone(),
        ));
    }
    // oracle.python — TODO for future
}

fn dynamic_to_json(val: &rhai::Dynamic) -> serde_json::Value {
    if let Ok(b) = val.as_bool() {
        serde_json::Value::from(b)
    } else if let Ok(i) = val.as_int() {
        serde_json::Value::from(i)
    } else if let Ok(f) = val.as_float() {
        serde_json::Value::from(f)
    } else if let Ok(s) = val.clone().into_string() {
        serde_json::Value::from(s)
    } else {
        serde_json::Value::Null
    }
}
