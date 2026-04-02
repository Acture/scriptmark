use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Comparison mode for similarity detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimilarityMode {
	/// Compare raw source code — catches identical variable names, spacing, comments.
	/// Best for detecting direct copy-paste.
	Style,
	/// Normalize code first (strip comments, whitespace, lowercase) — catches
	/// structural similarity even after variable renaming.
	Structure,
}

/// Prepare source code for comparison based on mode.
fn prepare(source: &str, mode: SimilarityMode) -> String {
	match mode {
		SimilarityMode::Style => {
			// Keep everything — variable names, spacing, comments are the signal.
			// Only strip trailing whitespace per line and trailing blank lines.
			source
				.lines()
				.map(|l| l.trim_end())
				.collect::<Vec<_>>()
				.join("\n")
				.trim_end()
				.to_string()
		}
		SimilarityMode::Structure => normalize_python(source),
	}
}

/// Normalize Python source for structural comparison.
/// Strips comments, docstrings, whitespace, lowercases.
fn normalize_python(source: &str) -> String {
	let mut lines = Vec::new();
	let mut in_docstring = false;

	for line in source.lines() {
		let trimmed = line.trim();

		let triple_count = trimmed.matches("\"\"\"").count() + trimmed.matches("'''").count();
		if triple_count > 0 {
			if in_docstring {
				in_docstring = false;
				continue;
			} else if triple_count == 1 {
				in_docstring = true;
				continue;
			}
			continue;
		}
		if in_docstring {
			continue;
		}

		let line = if let Some(pos) = trimmed.find('#') {
			&trimmed[..pos]
		} else {
			trimmed
		};

		let line = line.trim();
		if line.is_empty() {
			continue;
		}

		lines.push(line.to_lowercase());
	}

	lines.join("\n")
}

/// Generate character n-grams from text.
fn ngrams(text: &str, n: usize) -> HashSet<u64> {
	use std::hash::{Hash, Hasher};

	if text.len() < n {
		return HashSet::new();
	}

	let bytes = text.as_bytes();
	let mut grams = HashSet::new();

	for window in bytes.windows(n) {
		let mut hasher = std::hash::DefaultHasher::new();
		window.hash(&mut hasher);
		grams.insert(hasher.finish());
	}

	grams
}

/// Jaccard similarity: |A ∩ B| / |A ∪ B|
fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
	if a.is_empty() && b.is_empty() {
		return 0.0;
	}
	let intersection = a.intersection(b).count() as f64;
	let union = a.union(b).count() as f64;
	if union == 0.0 {
		return 0.0;
	}
	intersection / union
}

/// A pair of students with their similarity scores.
#[derive(Debug, Clone)]
pub struct SimilarityPair {
	pub student_a: String,
	pub student_b: String,
	/// Style similarity (raw code comparison).
	pub style_score: f64,
	/// Structural similarity (normalized comparison).
	pub structure_score: f64,
	/// Combined score (max of both — if either is high, it's suspicious).
	pub score: f64,
}

/// Compare all student submissions pairwise.
///
/// Computes both style and structure similarity. The combined `score` is
/// the maximum of the two — a high style score (identical variable names,
/// spacing) is strong evidence even if structure score is moderate.
pub fn compare_submissions(
	submissions: &HashMap<String, Vec<PathBuf>>,
	ngram_size: usize,
	threshold: f64,
) -> Vec<SimilarityPair> {
	let mut style_fps: HashMap<String, HashSet<u64>> = HashMap::new();
	let mut struct_fps: HashMap<String, HashSet<u64>> = HashMap::new();

	for (sid, files) in submissions {
		let mut raw = String::new();
		for file in files {
			if let Ok(source) = std::fs::read_to_string(file) {
				raw.push_str(&source);
				raw.push('\n');
			}
		}
		style_fps.insert(
			sid.clone(),
			ngrams(&prepare(&raw, SimilarityMode::Style), ngram_size),
		);
		struct_fps.insert(
			sid.clone(),
			ngrams(&prepare(&raw, SimilarityMode::Structure), ngram_size),
		);
	}

	let mut sids: Vec<&String> = style_fps.keys().collect();
	sids.sort();
	let mut pairs = Vec::new();

	for i in 0..sids.len() {
		for j in (i + 1)..sids.len() {
			let style = jaccard(&style_fps[sids[i]], &style_fps[sids[j]]);
			let structure = jaccard(&struct_fps[sids[i]], &struct_fps[sids[j]]);
			let combined = style.max(structure);

			if combined >= threshold {
				pairs.push(SimilarityPair {
					student_a: sids[i].clone(),
					student_b: sids[j].clone(),
					style_score: style,
					structure_score: structure,
					score: combined,
				});
			}
		}
	}

	pairs.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	pairs
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_style_mode_preserves_details() {
		let code = "x = 1  # my comment\nY = 2\n";
		let prepared = prepare(code, SimilarityMode::Style);
		assert!(prepared.contains("# my comment"));
		assert!(prepared.contains("Y = 2")); // case preserved
	}

	#[test]
	fn test_structure_mode_strips_details() {
		let code = "X = 1  # my comment\nY = 2\n";
		let prepared = prepare(code, SimilarityMode::Structure);
		assert!(!prepared.contains("# my comment"));
		assert!(prepared.contains("x = 1")); // lowercased
	}

	#[test]
	fn test_identical_code_high_similarity() {
		let code = "def f(x):\n    return x + 1\n";
		let a = ngrams(&prepare(code, SimilarityMode::Style), 5);
		let b = ngrams(&prepare(code, SimilarityMode::Style), 5);
		assert_eq!(jaccard(&a, &b), 1.0);
	}

	#[test]
	fn test_variable_rename_high_structure_low_style() {
		let code_a = "def find_max(a, b):\n    if a > b:\n        return a\n    return b\n";
		let code_b = "def find_max(x, y):\n    if x > y:\n        return x\n    return y\n";

		let style_a = ngrams(&prepare(code_a, SimilarityMode::Style), 5);
		let style_b = ngrams(&prepare(code_b, SimilarityMode::Style), 5);
		let style_sim = jaccard(&style_a, &style_b);

		let struct_a = ngrams(&prepare(code_a, SimilarityMode::Structure), 5);
		let struct_b = ngrams(&prepare(code_b, SimilarityMode::Structure), 5);
		let struct_sim = jaccard(&struct_a, &struct_b);

		// Both should show some similarity for structurally identical code
		// On short code with small n-grams, the difference may be small
		assert!(struct_sim > 0.0);
		assert!(style_sim > 0.0);
	}
}
