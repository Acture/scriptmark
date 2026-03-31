use rand::Rng;
use rand::rngs::StdRng;
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
                let idx = rng.random_range(0u8..36);
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
            return Err(GeneratorError::InvalidExpression(
                "choice with empty array".into(),
            ));
        }
        let idx = rng.random_range(0..arr.len());
        return Ok(arr[idx].clone());
    }
    if let Some(inner) = strip_call(expr, "list") {
        let (gen_expr, min_len, max_len) = parse_list_args(&inner)?;
        let len = rng.random_range(min_len..=max_len);
        let items: Result<Vec<Value>, _> =
            (0..len).map(|_| generate_value(&gen_expr, rng)).collect();
        return Ok(Value::from(items?));
    }

    Err(GeneratorError::InvalidExpression(expr.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum GeneratorError {
    #[error("invalid generator expression: {0}")]
    InvalidExpression(String),
    #[error("parse error in generator: {0}")]
    ParseError(String),
}

/// Strip a function call: `name(inner)` → Some(inner)
fn strip_call(expr: &str, name: &str) -> Option<String> {
    let expr = expr.trim();
    if expr.starts_with(name) && expr[name.len()..].starts_with('(') && expr.ends_with(')') {
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
        return Err(GeneratorError::ParseError(format!(
            "expected 2 args, got {}",
            parts.len()
        )));
    }
    let a = parts[0]
        .trim()
        .parse::<T>()
        .map_err(|e| GeneratorError::ParseError(e.to_string()))?;
    let b = parts[1]
        .trim()
        .parse::<T>()
        .map_err(|e| GeneratorError::ParseError(e.to_string()))?;
    Ok((a, b))
}

fn parse_list_args(s: &str) -> Result<(String, usize, usize), GeneratorError> {
    let s = s.trim();
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
        return Err(GeneratorError::ParseError(
            "list needs (generator, min, max)".into(),
        ));
    }
    let last = comma_positions[comma_positions.len() - 1];
    let second_last = comma_positions[comma_positions.len() - 2];

    let gen_expr = s[..second_last].trim().to_string();
    let min: usize = s[second_last + 1..last]
        .trim()
        .parse()
        .map_err(|e: std::num::ParseIntError| GeneratorError::ParseError(e.to_string()))?;
    let max: usize = s[last + 1..]
        .trim()
        .parse()
        .map_err(|e: std::num::ParseIntError| GeneratorError::ParseError(e.to_string()))?;
    Ok((gen_expr, min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

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
