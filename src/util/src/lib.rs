use rand::distr::uniform;
use std::error::Error;
use std::str::FromStr;
use rand::prelude::StdRng;
use rand::Rng;
use rand::SeedableRng;
use regex::Regex;

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
	let re = Regex::new(r"\d+").expect("正则表达式错误");
	re.find_iter(s)
		.filter_map(|mat| mat.as_str().parse::<T>().ok())
		.collect()
}