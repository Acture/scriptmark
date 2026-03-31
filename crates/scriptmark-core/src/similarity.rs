use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Normalize Python source code for comparison.
///
/// - Remove comments (lines starting with #)
/// - Remove docstrings (triple quotes) -- simplified: remove lines between triple quotes
/// - Collapse whitespace
/// - Lowercase
/// - Remove blank lines
fn normalize_python(source: &str) -> String {
    let mut lines = Vec::new();
    let mut in_docstring = false;

    for line in source.lines() {
        let trimmed = line.trim();

        // Toggle docstring state
        let triple_count = trimmed.matches("\"\"\"").count() + trimmed.matches("'''").count();
        if triple_count > 0 {
            if in_docstring {
                in_docstring = false;
                continue;
            } else if triple_count == 1 {
                in_docstring = true;
                continue;
            }
            // triple_count >= 2 means open+close on same line -- skip it
            continue;
        }
        if in_docstring {
            continue;
        }

        // Remove inline comments
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

/// Generate character n-grams from normalized text.
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

/// Jaccard similarity between two sets: |A ∩ B| / |A ∪ B|
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

/// A pair of students with their similarity score.
#[derive(Debug, Clone)]
pub struct SimilarityPair {
    pub student_a: String,
    pub student_b: String,
    pub score: f64,
}

/// Compare all student submissions pairwise.
///
/// Returns pairs sorted by similarity (highest first), filtered by threshold.
pub fn compare_submissions(
    submissions: &HashMap<String, Vec<PathBuf>>,
    ngram_size: usize,
    threshold: f64,
) -> Vec<SimilarityPair> {
    // Build fingerprints per student
    let mut fingerprints: HashMap<String, HashSet<u64>> = HashMap::new();

    for (sid, files) in submissions {
        let mut combined = String::new();
        for file in files {
            if let Ok(source) = std::fs::read_to_string(file) {
                combined.push_str(&normalize_python(&source));
                combined.push('\n');
            }
        }
        fingerprints.insert(sid.clone(), ngrams(&combined, ngram_size));
    }

    // Pairwise comparison
    let mut sids: Vec<&String> = fingerprints.keys().collect();
    sids.sort(); // deterministic ordering
    let mut pairs = Vec::new();

    for i in 0..sids.len() {
        for j in (i + 1)..sids.len() {
            let score = jaccard(&fingerprints[sids[i]], &fingerprints[sids[j]]);
            if score >= threshold {
                pairs.push(SimilarityPair {
                    student_a: sids[i].clone(),
                    student_b: sids[j].clone(),
                    score,
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
    fn test_normalize_python() {
        let source = r#"
# This is a comment
def foo():
    """Docstring"""
    x = 1  # inline comment
    return x
"#;
        let normalized = normalize_python(source);
        assert!(!normalized.contains('#'));
        assert!(!normalized.contains("docstring"));
        assert!(normalized.contains("def foo():"));
        assert!(normalized.contains("x = 1"));
    }

    #[test]
    fn test_identical_code_high_similarity() {
        let code = "def f(x):\n    return x + 1\n";
        let a = ngrams(&normalize_python(code), 5);
        let b = ngrams(&normalize_python(code), 5);
        assert_eq!(jaccard(&a, &b), 1.0);
    }

    #[test]
    fn test_different_code_low_similarity() {
        let code_a = "def sort_list(lst):\n    return sorted(lst)\n";
        let code_b = "class Database:\n    def __init__(self):\n        self.data = {}\n";
        let a = ngrams(&normalize_python(code_a), 5);
        let b = ngrams(&normalize_python(code_b), 5);
        assert!(jaccard(&a, &b) < 0.3);
    }

    #[test]
    fn test_similar_code_medium_similarity() {
        let code_a = "def find_max(a, b):\n    if a > b:\n        return a\n    return b\n";
        let code_b = "def find_max(x, y):\n    if x > y:\n        return x\n    return y\n";
        let a = ngrams(&normalize_python(code_a), 5);
        let b = ngrams(&normalize_python(code_b), 5);
        // Variable rename on short code still shares structural n-grams
        let score = jaccard(&a, &b);
        assert!(score > 0.15, "expected > 0.15, got {score}");
    }
}
