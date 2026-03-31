# Parametrize + Oracle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teachers define generator rules for test inputs + an oracle (reference implementation or expression), ScriptMark auto-generates N test cases and verifies student output against the oracle.

**Architecture:** New `Parametrize` and `Oracle` structs in spec model. New `generator.rs` module in scriptmark-runner that parses generator expressions (`int(-100,100)`, `list(int(0,10), 1, 5)`, etc.) and produces random `serde_json::Value`s with a seeded RNG. The orchestrator expands parametrized cases into concrete `TestCase`s before execution. Oracle reference implementation reuses `PythonExecutor` to run the teacher's code with the same args.

**Tech Stack:** `rand` crate (seeded RNG), existing `rhai` + `python_checker` for oracle expressions, existing `PythonExecutor` for reference implementation oracle.

---

### File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/scriptmark-core/src/models/spec.rs` | Modify | Add `Parametrize`, `Oracle`, `GeneratorExpr` structs |
| `crates/scriptmark-runner/src/generator.rs` | Create | Parse generator expressions, produce random values |
| `crates/scriptmark-runner/src/expander.rs` | Create | Expand parametrized cases into concrete TestCases |
| `crates/scriptmark-runner/src/oracle.rs` | Create | Run oracle (reference impl / rhai / checker) to get expected value |
| `crates/scriptmark-runner/src/lib.rs` | Modify | Add `pub mod generator; pub mod expander; pub mod oracle;` |
| `crates/scriptmark-runner/src/orchestrator.rs` | Modify | Call expander before running cases |
| `crates/scriptmark-runner/Cargo.toml` | Modify | Add `rand` dependency |
| `Cargo.toml` | Modify | Add `rand` to workspace dependencies |
| `crates/scriptmark-runner/tests/integration.rs` | Modify | Add parametrize integration tests |

---

### Task 1: Spec Model — Parametrize + Oracle structs

**Files:**
- Modify: `crates/scriptmark-core/src/models/spec.rs`

- [ ] **Step 1: Write the test for TOML parsing**

In `crates/scriptmark-core/src/spec_loader.rs`, add:

```rust
#[test]
fn test_load_parametrized_spec() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_param.toml");
    std::fs::write(
        &path,
        r#"
[meta]
name = "random_max"
file = "lab5.py"
function = "find_larger_number"
language = "python"

[[cases]]
name = "random pairs"

[cases.parametrize]
count = 20
seed = 42

[cases.parametrize.args]
a = "int(-100, 100)"
b = "int(-100, 100)"

[cases.parametrize.oracle]
reference = "solutions/lab5.py"
"#,
    )
    .unwrap();

    let spec = load_spec(&path).unwrap();
    assert_eq!(spec.cases.len(), 1);
    let case = &spec.cases[0];
    let param = case.parametrize.as_ref().unwrap();
    assert_eq!(param.count, 20);
    assert_eq!(param.seed, Some(42));
    assert_eq!(param.args.len(), 2);
    assert!(param.oracle.reference.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p scriptmark-core test_load_parametrized_spec -- --nocapture`
Expected: FAIL — `parametrize` field doesn't exist on TestCase yet.

- [ ] **Step 3: Add structs to spec.rs**

Add to `crates/scriptmark-core/src/models/spec.rs`:

```rust
/// Oracle — how to determine the expected output for generated inputs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Oracle {
    /// Teacher's reference implementation file. Same function name, compare outputs.
    #[serde(default)]
    pub reference: Option<String>,
    /// Rhai expression computing expected value from generated args.
    #[serde(default)]
    pub rhai: Option<String>,
    /// Built-in checker name (just verifies a property, no expected value).
    #[serde(default)]
    pub check: Option<String>,
    /// Python script oracle.
    #[serde(default)]
    pub python: Option<String>,
}

/// Parametrize configuration — auto-generate test cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parametrize {
    /// Number of test cases to generate.
    pub count: usize,
    /// Random seed for reproducibility.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Generator expressions per argument. Key = arg name, Value = generator string.
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
    /// How to determine the expected output.
    #[serde(default)]
    pub oracle: Oracle,
}
```

Add `parametrize` field to `TestCase`:

```rust
pub struct TestCase {
    // ... existing fields ...

    /// Parametrize configuration for auto-generating test cases.
    #[serde(default)]
    pub parametrize: Option<Parametrize>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p scriptmark-core test_load_parametrized_spec -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/scriptmark-core/src/models/spec.rs crates/scriptmark-core/src/spec_loader.rs
git commit -m "feat(spec): add Parametrize and Oracle model structs"
```

---

### Task 2: Generator — Parse expressions and produce random values

**Files:**
- Create: `crates/scriptmark-runner/src/generator.rs`
- Modify: `crates/scriptmark-runner/src/lib.rs`
- Modify: `crates/scriptmark-runner/Cargo.toml`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Add rand dependency**

In workspace `Cargo.toml`, add under `[workspace.dependencies]`:
```toml
rand = "0.9"
```

In `crates/scriptmark-runner/Cargo.toml`, add under `[dependencies]`:
```toml
rand = { workspace = true }
```

- [ ] **Step 2: Write failing tests for generator**

Create `crates/scriptmark-runner/src/generator.rs`:

```rust
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde_json::Value;

/// Parse a generator expression and produce a random value.
///
/// Supported expressions:
/// - `int(min, max)` — random integer in [min, max]
/// - `float(min, max)` — random float in [min, max]
/// - `bool()` — random boolean
/// - `str(min_len, max_len)` — random alphanumeric string
/// - `choice([v1, v2, ...])` — random pick from JSON array
/// - `list(gen_expr, min_len, max_len)` — list of random values
pub fn generate_value(expr: &str, rng: &mut StdRng) -> Result<Value, GeneratorError> {
    let expr = expr.trim();
    // ... implementation in step 3
    todo!()
}

#[derive(Debug, thiserror::Error)]
pub enum GeneratorError {
    #[error("invalid generator expression: {0}")]
    InvalidExpression(String),
    #[error("parse error in generator: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    #[test]
    fn test_int_generator() {
        let mut rng = seeded_rng();
        let val = generate_value("int(-100, 100)", &mut rng).unwrap();
        let n = val.as_i64().unwrap();
        assert!((-100..=100).contains(&n));
    }

    #[test]
    fn test_float_generator() {
        let mut rng = seeded_rng();
        let val = generate_value("float(0.0, 1.0)", &mut rng).unwrap();
        let f = val.as_f64().unwrap();
        assert!((0.0..=1.0).contains(&f));
    }

    #[test]
    fn test_bool_generator() {
        let mut rng = seeded_rng();
        let val = generate_value("bool()", &mut rng).unwrap();
        assert!(val.is_boolean());
    }

    #[test]
    fn test_str_generator() {
        let mut rng = seeded_rng();
        let val = generate_value("str(3, 10)", &mut rng).unwrap();
        let s = val.as_str().unwrap();
        assert!(s.len() >= 3 && s.len() <= 10);
    }

    #[test]
    fn test_choice_generator() {
        let mut rng = seeded_rng();
        let val = generate_value(r#"choice(["a", "b", "c"])"#, &mut rng).unwrap();
        let s = val.as_str().unwrap();
        assert!(["a", "b", "c"].contains(&s));
    }

    #[test]
    fn test_list_generator() {
        let mut rng = seeded_rng();
        let val = generate_value("list(int(0, 10), 3, 5)", &mut rng).unwrap();
        let arr = val.as_array().unwrap();
        assert!(arr.len() >= 3 && arr.len() <= 5);
        for item in arr {
            let n = item.as_i64().unwrap();
            assert!((0..=10).contains(&n));
        }
    }

    #[test]
    fn test_seed_reproducibility() {
        let mut rng1 = StdRng::seed_from_u64(42);
        let mut rng2 = StdRng::seed_from_u64(42);
        let v1 = generate_value("int(0, 1000)", &mut rng1).unwrap();
        let v2 = generate_value("int(0, 1000)", &mut rng2).unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_invalid_expression() {
        let mut rng = seeded_rng();
        assert!(generate_value("invalid()", &mut rng).is_err());
    }
}
```

- [ ] **Step 3: Implement the generator**

Replace the `todo!()` in `generate_value` with the actual parser:

```rust
pub fn generate_value(expr: &str, rng: &mut StdRng) -> Result<Value, GeneratorError> {
    let expr = expr.trim();

    if let Some(inner) = strip_call(expr, "int") {
        let (min, max) = parse_two_nums::<i64>(&inner)?;
        let val = rng.random_range(min..=max);
        return Ok(Value::from(val));
    }
    if let Some(inner) = strip_call(expr, "float") {
        let (min, max) = parse_two_nums::<f64>(&inner)?;
        let val: f64 = rng.random_range(min..=max);
        return Ok(Value::from(val));
    }
    if strip_call(expr, "bool").is_some() {
        return Ok(Value::from(rng.random_bool(0.5)));
    }
    if let Some(inner) = strip_call(expr, "str") {
        let (min_len, max_len) = parse_two_nums::<usize>(&inner)?;
        let len = rng.random_range(min_len..=max_len);
        let s: String = (0..len)
            .map(|_| {
                let idx = rng.random_range(0..36);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'a' + idx - 10) as char
                }
            })
            .collect();
        return Ok(Value::from(s));
    }
    if let Some(inner) = strip_call(expr, "choice") {
        let arr: Vec<Value> = serde_json::from_str(&inner)
            .map_err(|e| GeneratorError::ParseError(format!("choice array: {e}")))?;
        if arr.is_empty() {
            return Err(GeneratorError::InvalidExpression("choice with empty array".into()));
        }
        let idx = rng.random_range(0..arr.len());
        return Ok(arr[idx].clone());
    }
    if let Some(inner) = strip_call(expr, "list") {
        // list(gen_expr, min_len, max_len)
        // Find the last two comma-separated numbers, rest is the inner generator
        let (gen_expr, min_len, max_len) = parse_list_args(&inner)?;
        let len = rng.random_range(min_len..=max_len);
        let items: Result<Vec<Value>, _> = (0..len)
            .map(|_| generate_value(&gen_expr, rng))
            .collect();
        return Ok(Value::from(items?));
    }

    Err(GeneratorError::InvalidExpression(expr.to_string()))
}

/// Strip a function call: `name(inner)` → Some(inner)
fn strip_call(expr: &str, name: &str) -> Option<String> {
    let expr = expr.trim();
    if expr.starts_with(name)
        && expr[name.len()..].starts_with('(')
        && expr.ends_with(')')
    {
        Some(expr[name.len() + 1..expr.len() - 1].to_string())
    } else {
        None
    }
}

fn parse_two_nums<T: std::str::FromStr>(s: &str) -> Result<(T, T), GeneratorError>
where
    T::Err: std::fmt::Display,
{
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(GeneratorError::ParseError(format!("expected 2 args, got {}", parts.len())));
    }
    let a = parts[0].trim().parse::<T>()
        .map_err(|e| GeneratorError::ParseError(e.to_string()))?;
    let b = parts[1].trim().parse::<T>()
        .map_err(|e| GeneratorError::ParseError(e.to_string()))?;
    Ok((a, b))
}

fn parse_list_args(s: &str) -> Result<(String, usize, usize), GeneratorError> {
    // Find the last two comma-separated tokens that are numbers
    let s = s.trim();
    // Scan from the end to find the last two commas at depth 0
    let mut depth = 0i32;
    let mut comma_positions = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => comma_positions.push(i),
            _ => {}
        }
    }
    if comma_positions.len() < 2 {
        return Err(GeneratorError::ParseError("list needs (generator, min, max)".into()));
    }
    let last = comma_positions[comma_positions.len() - 1];
    let second_last = comma_positions[comma_positions.len() - 2];

    let gen_expr = s[..second_last].trim().to_string();
    let min: usize = s[second_last + 1..last].trim().parse()
        .map_err(|e: std::num::ParseIntError| GeneratorError::ParseError(e.to_string()))?;
    let max: usize = s[last + 1..].trim().parse()
        .map_err(|e: std::num::ParseIntError| GeneratorError::ParseError(e.to_string()))?;
    Ok((gen_expr, min, max))
}
```

- [ ] **Step 4: Add `pub mod generator;` to lib.rs**

- [ ] **Step 5: Run tests**

Run: `cargo test -p scriptmark-runner generator`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/scriptmark-runner/Cargo.toml crates/scriptmark-runner/src/generator.rs crates/scriptmark-runner/src/lib.rs
git commit -m "feat(runner): add generator module for parametrize expressions"
```

---

### Task 3: Expander — Turn parametrized cases into concrete TestCases

**Files:**
- Create: `crates/scriptmark-runner/src/expander.rs`

- [ ] **Step 1: Write failing tests**

```rust
use rand::rngs::StdRng;
use rand::SeedableRng;
use scriptmark_core::models::{TestCase, Parametrize, Oracle};

/// Expand a parametrized TestCase into N concrete TestCases.
/// Non-parametrized cases are returned as-is.
pub fn expand_cases(cases: &[TestCase]) -> Vec<TestCase> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_non_parametrized_passthrough() {
        let cases = vec![TestCase {
            name: "simple".into(),
            args: vec![serde_json::json!(3), serde_json::json!(5)],
            expect: Some(serde_json::json!(5)),
            ..Default::default()
        }];
        let expanded = expand_cases(&cases);
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].name, "simple");
    }

    #[test]
    fn test_parametrized_expansion() {
        let mut args = HashMap::new();
        args.insert("a".into(), "int(0, 10)".into());
        args.insert("b".into(), "int(0, 10)".into());

        let cases = vec![TestCase {
            name: "random test".into(),
            parametrize: Some(Parametrize {
                count: 5,
                seed: Some(42),
                args,
                oracle: Oracle::default(),
            }),
            ..Default::default()
        }];
        let expanded = expand_cases(&cases);
        assert_eq!(expanded.len(), 5);
        // Each case should have args [a, b] as positional
        for (i, case) in expanded.iter().enumerate() {
            assert_eq!(case.name, format!("random test [{}]", i));
            assert_eq!(case.args.len(), 2);
            assert!(case.parametrize.is_none()); // expanded cases are concrete
        }
    }

    #[test]
    fn test_seed_reproducibility() {
        let mut args = HashMap::new();
        args.insert("x".into(), "int(0, 1000)".into());

        let cases = vec![TestCase {
            name: "seeded".into(),
            parametrize: Some(Parametrize {
                count: 3,
                seed: Some(99),
                args,
                oracle: Oracle::default(),
            }),
            ..Default::default()
        }];
        let run1 = expand_cases(&cases);
        let run2 = expand_cases(&cases);
        assert_eq!(run1[0].args, run2[0].args);
        assert_eq!(run1[1].args, run2[1].args);
    }
}
```

- [ ] **Step 2: Implement expander**

```rust
use rand::rngs::StdRng;
use rand::SeedableRng;
use scriptmark_core::models::TestCase;

use crate::generator::generate_value;

pub fn expand_cases(cases: &[TestCase]) -> Vec<TestCase> {
    let mut result = Vec::new();

    for case in cases {
        if let Some(param) = &case.parametrize {
            let seed = param.seed.unwrap_or(0);
            let mut rng = StdRng::seed_from_u64(seed);

            // Sort arg names for deterministic positional ordering
            let mut arg_names: Vec<&String> = param.args.keys().collect();
            arg_names.sort();

            for i in 0..param.count {
                let mut args = Vec::new();
                for name in &arg_names {
                    let expr = &param.args[*name];
                    match generate_value(expr, &mut rng) {
                        Ok(val) => args.push(val),
                        Err(_) => args.push(serde_json::Value::Null),
                    }
                }

                result.push(TestCase {
                    name: format!("{} [{}]", case.name, i),
                    id: None,
                    args,
                    expect: None, // oracle determines expected value
                    expect_error: None,
                    stdin: None,
                    expected_stdout: None,
                    check: case.check.clone(),
                    timeout: case.timeout,
                    parametrize: None, // expanded = concrete
                });
            }
        } else {
            result.push(case.clone());
        }
    }

    result
}
```

- [ ] **Step 3: Add `pub mod expander;` to lib.rs**

- [ ] **Step 4: Add `Default` derive to TestCase**

In `spec.rs`, add `#[derive(Default)]` or implement it manually for TestCase.

- [ ] **Step 5: Run tests**

Run: `cargo test -p scriptmark-runner expander`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add crates/scriptmark-runner/src/expander.rs crates/scriptmark-runner/src/lib.rs crates/scriptmark-core/src/models/spec.rs
git commit -m "feat(runner): add expander to turn parametrized cases into concrete test cases"
```

---

### Task 4: Oracle — Run reference implementation to get expected values

**Files:**
- Create: `crates/scriptmark-runner/src/oracle.rs`

- [ ] **Step 1: Write oracle module**

The oracle module takes a generated `TestCase` (with args but no expect) and fills in the expected value by running the teacher's reference implementation.

```rust
use std::path::Path;
use scriptmark_core::models::{TestCase, TestSpec, StudentFile};
use crate::checker::rhai_checker::RhaiChecker;
use crate::checker::{CheckInput, Checker};
use crate::python::PythonExecutor;

use scriptmark_core::models::spec::Oracle;

/// Resolve the expected value for a parametrized case using the oracle.
///
/// For `oracle.reference`: runs teacher's code with same function + args, uses return value as expected.
/// For `oracle.rhai`: evaluates expression with arg names in scope, uses result as expected.
/// For `oracle.check`: no expected value needed, just set the checker.
pub async fn resolve_oracle(
    case: &mut TestCase,
    oracle: &Oracle,
    spec: &TestSpec,
    executor: &PythonExecutor,
    arg_names: &[String],
) {
    if let Some(ref_path) = &oracle.reference {
        // Run teacher's reference implementation
        let ref_file = StudentFile {
            path: Path::new(ref_path).to_path_buf(),
            language: "python".to_string(),
        };
        let ref_spec = TestSpec {
            meta: spec.meta.clone(),
            vars: Default::default(),
            setup: vec![],
            cases: vec![],
        };
        let result = executor
            .execute_case(&[ref_file], &ref_spec, case, 10)
            .await;
        if let Some(actual) = &result.actual {
            case.expect = serde_json::from_str(actual).ok();
        }
    } else if let Some(rhai_expr) = &oracle.rhai {
        // Build a Rhai scope with arg names → arg values
        let mut engine = rhai::Engine::new();
        let mut scope = rhai::Scope::new();
        for (i, name) in arg_names.iter().enumerate() {
            if let Some(val) = case.args.get(i) {
                scope.push_dynamic(name.as_str(), crate::checker::rhai_checker::json_to_dynamic(val));
            }
        }
        if let Ok(result) = engine.eval_with_scope::<rhai::Dynamic>(&mut scope, rhai_expr) {
            case.expect = Some(dynamic_to_json(&result));
        }
    } else if let Some(check_name) = &oracle.check {
        // Just set the checker, no expected value
        case.check = Some(scriptmark_core::models::CheckMethod::Builtin(check_name.clone()));
    }
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
```

- [ ] **Step 2: Make `json_to_dynamic` public in rhai_checker.rs**

Change `fn json_to_dynamic` to `pub fn json_to_dynamic` in `crates/scriptmark-runner/src/checker/rhai_checker.rs`.

- [ ] **Step 3: Add `pub mod oracle;` to lib.rs**

- [ ] **Step 4: Commit**

```bash
git add crates/scriptmark-runner/src/oracle.rs crates/scriptmark-runner/src/lib.rs crates/scriptmark-runner/src/checker/rhai_checker.rs
git commit -m "feat(runner): add oracle module for parametrize expected value resolution"
```

---

### Task 5: Wire up in Orchestrator

**Files:**
- Modify: `crates/scriptmark-runner/src/orchestrator.rs`

- [ ] **Step 1: Update orchestrator to expand parametrized cases and resolve oracles**

In `run_student`, between step 2 (setup) and step 3 (run cases), add:

```rust
// 2.5. Expand parametrized cases
let expanded_cases = crate::expander::expand_cases(&spec.cases);

// 2.6. Resolve oracles for parametrized cases
let mut final_cases = Vec::new();
for mut case in expanded_cases {
    // Find the original parametrized case to get the oracle
    let original = spec.cases.iter().find(|c| case.name.starts_with(&c.name));
    if let Some(orig) = original {
        if let Some(param) = &orig.parametrize {
            let mut arg_names: Vec<String> = param.args.keys().cloned().collect();
            arg_names.sort();
            crate::oracle::resolve_oracle(
                &mut case, &param.oracle, spec, executor, &arg_names,
            ).await;
        }
    }
    final_cases.push(case);
}
```

Then use `final_cases` instead of `spec.cases` in the execution loop.

- [ ] **Step 2: Run all existing tests to verify nothing breaks**

Run: `cargo test`
Expected: ALL 40+ existing tests PASS

- [ ] **Step 3: Commit**

```bash
git add crates/scriptmark-runner/src/orchestrator.rs
git commit -m "feat(orchestrator): wire up parametrize expansion and oracle resolution"
```

---

### Task 6: Integration Test — Full Parametrize + Oracle End-to-End

**Files:**
- Modify: `crates/scriptmark-runner/tests/integration.rs`

- [ ] **Step 1: Write integration test with reference oracle**

```rust
#[tokio::test]
async fn test_parametrize_with_reference_oracle() {
    let dir = tempfile::tempdir().unwrap();

    // Student code — correct implementation
    std::fs::write(
        dir.path().join("alice_lab5.py"),
        "def find_max(a, b):\n    return max(a, b)\n",
    ).unwrap();

    // Teacher reference implementation (the oracle)
    let solutions_dir = dir.path().join("solutions");
    std::fs::create_dir(&solutions_dir).unwrap();
    std::fs::write(
        solutions_dir.join("lab5.py"),
        "def find_max(a, b):\n    return max(a, b)\n",
    ).unwrap();

    let spec: TestSpec = toml::from_str(&format!(
        r#"
[meta]
name = "parametrized_max"
file = "lab5.py"
function = "find_max"
language = "python"

[[cases]]
name = "random max"

[cases.parametrize]
count = 10
seed = 42

[cases.parametrize.args]
a = "int(-50, 50)"
b = "int(-50, 50)"

[cases.parametrize.oracle]
reference = "{}/solutions/lab5.py"
"#,
        dir.path().display()
    )).unwrap();

    let executor = PythonExecutor::new();
    let submissions = SubmissionSet {
        by_student: HashMap::from([(
            "alice".to_string(),
            vec![StudentFile {
                path: dir.path().join("alice_lab5.py"),
                language: "python".to_string(),
            }],
        )]),
    };

    let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
    let alice = &results["alice"];

    assert_eq!(alice.total_cases(), 10);
    assert_eq!(alice.total_passed(), 10, "Correct implementation should pass all generated cases");
}

#[tokio::test]
async fn test_parametrize_with_rhai_oracle() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("alice_lab5.py"),
        "def find_max(a, b):\n    return max(a, b)\n",
    ).unwrap();

    let spec: TestSpec = toml::from_str(
        r#"
[meta]
name = "rhai_oracle_max"
file = "lab5.py"
function = "find_max"
language = "python"

[[cases]]
name = "rhai oracle"

[cases.parametrize]
count = 5
seed = 123

[cases.parametrize.args]
a = "int(0, 100)"
b = "int(0, 100)"

[cases.parametrize.oracle]
rhai = "if a >= b { a } else { b }"
"#,
    ).unwrap();

    let executor = PythonExecutor::new();
    let submissions = SubmissionSet {
        by_student: HashMap::from([(
            "alice".to_string(),
            vec![StudentFile {
                path: dir.path().join("alice_lab5.py"),
                language: "python".to_string(),
            }],
        )]),
    };

    let results = orchestrator::run_all(&submissions, &[spec], &executor, 10, Some(1)).await;
    let alice = &results["alice"];

    assert_eq!(alice.total_cases(), 5);
    assert_eq!(alice.total_passed(), 5, "Correct max implementation should match Rhai oracle");
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test -p scriptmark-runner --test integration`
Expected: ALL PASS

- [ ] **Step 3: Run full suite**

Run: `cargo fmt && cargo clippy --all-targets && cargo test`
Expected: ALL PASS, 0 warnings

- [ ] **Step 4: Commit**

```bash
git add crates/scriptmark-runner/tests/integration.rs
git commit -m "test: add parametrize + oracle integration tests"
```

---

### Verification

After all tasks:

1. `cargo test` — all tests pass (40+ existing + ~15 new)
2. `cargo clippy --all-targets` — 0 warnings
3. Manual CLI test:

```bash
# Create a TOML spec with parametrize + reference oracle
scriptmark grade /tmp/test-submissions -t /tmp/test-specs --curve sqrt
```

Example TOML that should work end-to-end:

```toml
[meta]
name = "parametrized_max"
file = "lab5.py"
function = "find_larger_number"
language = "python"

# Static cases
[[cases]]
name = "basic"
args = [3, 5]
expect = 5

# Auto-generated cases with teacher reference
[[cases]]
name = "random pairs"

[cases.parametrize]
count = 20
seed = 42

[cases.parametrize.args]
a = "int(-100, 100)"
b = "int(-100, 100)"

[cases.parametrize.oracle]
reference = "solutions/lab5.py"
```
