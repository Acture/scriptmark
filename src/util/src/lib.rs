use rand::distr::uniform;
use rand::prelude::StdRng;
use rand::Rng;
use rand::SeedableRng;
use regex::Regex;
use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;

pub fn generate<T: uniform::SampleUniform + std::cmp::PartialOrd + Clone>(
    seed: u64,
    size: usize,
    begin: T,
    end: T,
) -> Vec<T> {
    let mut rng = StdRng::seed_from_u64(seed);

    (0..size)
        .map(|_| rng.random_range(begin.clone()..=end.clone()))
        .collect()
}


fn calculate_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

pub fn calculate_hash_from_file(file_path: &std::path::Path) -> Result<u64, Box<dyn Error>> {
    let content = std::fs::read_to_string(file_path)?;
    Ok(calculate_hash(&content))
}

pub fn extract_numbers<T: FromStr + Copy>(s: &str) -> Vec<T>
where
    <T as FromStr>::Err: std::fmt::Debug,
{
    // Changed regex pattern to match both integers and floating-point numbers
    // \d+(\.\d+)? matches integers and numbers with decimal points
    let re = Regex::new(r"-?\d+(\.\d+)?").expect("正则表达式错误");
    re.find_iter(s)
        .filter_map(|mat| mat.as_str().parse::<T>().ok())
        .collect()
}

pub fn detect_type(value: &dyn Any) -> (String, String) {
    // Check primitive types
    if value.is::<String>() {
        return (
            "String".to_string(),
            value.downcast_ref::<String>().unwrap().to_string(),
        );
    }
    if value.is::<i32>() {
        return (
            "i32".to_string(),
            value.downcast_ref::<i32>().unwrap().to_string(),
        );
    }
    if value.is::<u32>() {
        return (
            "u32".to_string(),
            value.downcast_ref::<u32>().unwrap().to_string(),
        );
    }
    if value.is::<f64>() {
        return (
            "f64".to_string(),
            value.downcast_ref::<f64>().unwrap().to_string(),
        );
    }
    if value.is::<bool>() {
        return (
            "bool".to_string(),
            value.downcast_ref::<bool>().unwrap().to_string(),
        );
    }

    // Check common collections
    if value.is::<Vec<String>>() {
        return (
            "Vec<String>".to_string(),
            format!("{:?}", value.downcast_ref::<Vec<String>>().unwrap()),
        );
    }
    if value.is::<Vec<u8>>() {
        return (
            "Vec<u8>".to_string(),
            format!("{:?}", value.downcast_ref::<Vec<u8>>().unwrap()),
        );
    }
    if value.is::<Vec<i32>>() {
        return (
            "Vec<i32>".to_string(),
            format!("{:?}", value.downcast_ref::<Vec<i32>>().unwrap()),
        );
    }
    if value.is::<Vec<f64>>() {
        return (
            "Vec<f64>".to_string(),
            format!("{:?}", value.downcast_ref::<Vec<f64>>().unwrap()),
        );
    }

    // Check other common types
    if value.is::<HashMap<String, String>>() {
        return (
            "HashMap<String, String>".to_string(),
            format!(
                "{:?}",
                value.downcast_ref::<HashMap<String, String>>().unwrap()
            ),
        );
    }
    if value.is::<Option<String>>() {
        return (
            "Option<String>".to_string(),
            format!("{:?}", value.downcast_ref::<Option<String>>().unwrap()),
        );
    }
    if value.is::<Result<String, String>>() {
        return (
            "Result<String, String>".to_string(),
            format!(
                "{:?}",
                value.downcast_ref::<Result<String, String>>().unwrap()
            ),
        );
    }

    ("Unknown".to_string(), format!("{:?}", value.type_id()))
}
