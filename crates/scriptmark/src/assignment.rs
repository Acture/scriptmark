//! What an assignment is graded on: its identity, its items and its grading policy, read
//! from `assignment.toml` and settled against the specs before any student runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::grading::Policy;
use crate::models::{Assignment, AttemptPolicy, GradingConfig, GradingItem, TestSpec};

/// An `assignment.toml`, or the defaults when there is none.
#[derive(Debug, Clone)]
pub struct Declared {
	pub assignment: Assignment,
	pub attempt_policy: AttemptPolicy,
	pub grading: GradingConfig,
	pub matching: crate::matching::Config,
	/// Where it was read from; `None` when there was no file.
	pub path: Option<PathBuf>,
}

/// Load `assignment.toml`, explicitly or from beside the tests directory.
///
/// An explicit path that cannot be read or parsed is an error; an absent default is not.
pub fn load(explicit: Option<&Path>, tests_dir: &Path) -> Result<Declared> {
	let path = match explicit {
		Some(path) => Some(path.to_path_buf()),
		None => [tests_dir.parent(), Some(tests_dir)]
			.into_iter()
			.flatten()
			.map(|dir| dir.join("assignment.toml"))
			.find(|candidate| candidate.is_file()),
	};

	let Some(path) = path else {
		// Fall back to the directory name, which is what the db session has always used.
		let name = tests_dir
			.parent()
			.and_then(|p| p.file_name())
			.or_else(|| tests_dir.file_name())
			.and_then(|n| n.to_str())
			.unwrap_or("unknown");
		return Ok(Declared {
			assignment: Assignment::named(name),
			attempt_policy: AttemptPolicy::default(),
			grading: GradingConfig::default(),
			matching: crate::matching::Config::default(),
			path: None,
		});
	};

	let config = crate::spec_loader::load_assignment_config(&path)
		.with_context(|| format!("Failed to load {}", path.display()))?;
	Ok(Declared {
		assignment: Assignment {
			name: config.assignment.name,
			canvas_course_id: config.assignment.canvas_course_id,
			canvas_assignment_id: config.assignment.canvas_assignment_id,
			items: config.items.into_iter().map(GradingItem::from).collect(),
		},
		attempt_policy: config.assignment.attempt_policy,
		grading: config.grading,
		matching: config.matching,
		path: Some(path),
	})
}

/// Settle the items against the specs that were loaded, and compile the policy.
///
/// With no items declared, one is derived per spec at 1 point, and the policy records
/// that. Every problem is collected into one refusal, so a teacher fixes the file once.
pub fn settle(
	assignment: &mut Assignment,
	grading: &GradingConfig,
	specs: &[TestSpec],
) -> Result<Policy> {
	let mut problems = Vec::new();

	let mut names = BTreeSet::new();
	for spec in specs {
		if !names.insert(spec.meta.name.as_str()) {
			problems.push(format!(
				"two test specs are named '{}'; an item's results must come from one",
				spec.meta.name
			));
		}
	}

	let derived = assignment.items.is_empty();
	if derived {
		assignment.items = specs
			.iter()
			.map(|spec| GradingItem::new(&spec.meta.name))
			.collect();
	} else {
		let mut ids = BTreeSet::new();
		for item in &assignment.items {
			if !ids.insert(item.id.as_str()) {
				problems.push(format!("item '{}' is declared twice", item.id));
			}
			if !names.contains(item.id.as_str()) {
				problems.push(format!("item '{}' has no test spec", item.id));
			}
		}
		for spec in specs {
			if !ids.contains(spec.meta.name.as_str()) {
				problems.push(format!(
					"test spec '{}' is not a declared item; add it to [[items]]",
					spec.meta.name
				));
			}
		}
	}

	if assignment.items.iter().map(|i| i.points).sum::<u32>() + grading.lint_points.unwrap_or(0)
		== 0
	{
		problems.push("the items are worth 0 points in total".into());
	}
	if grading.lint_points.is_some() && !specs.iter().any(|s| s.lint.is_some()) {
		problems.push("[grading] lint_points is set, but no test spec declares [lint]".into());
	}

	match Policy::compile(grading.clone(), derived) {
		Ok(policy) if problems.is_empty() => Ok(policy),
		Ok(_) => bail!("refusing to grade: {}", problems.join("; ")),
		Err(e) => {
			problems.extend(e.0);
			bail!("refusing to grade: {}", problems.join("; "))
		}
	}
}

/// The `[[items]]` block a derived assignment amounts to, for the teacher to paste.
pub fn items_toml(items: &[GradingItem]) -> String {
	items
		.iter()
		.map(|item| {
			format!(
				"[[items]]\nid = {:?}\npoints = {}\naggregation = \"proportional\"\n",
				item.id, item.points
			)
		})
		.collect::<Vec<_>>()
		.join("\n")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::AssignmentConfig;

	fn spec(name: &str, lint: bool) -> TestSpec {
		let lint = if lint {
			"[lint]\ncommand = \"ruff check {file}\"\n"
		} else {
			""
		};
		crate::spec_loader::load_spec_str(
			&format!(
				"[meta]\nname = \"{name}\"\nfile = \"a.py\"\nfunction = \"f\"\nlanguage = \"python\"\n{lint}\n\
				 [[cases]]\nname = \"c\"\nexpect = 1\n"
			),
			Path::new("."),
		)
		.unwrap()
	}

	fn parse(toml: &str) -> Result<AssignmentConfig, toml::de::Error> {
		toml::from_str(&format!("[assignment]\nname = \"hw\"\n{toml}"))
	}

	fn settle_toml(toml: &str, specs: &[TestSpec]) -> Result<Policy> {
		let config = parse(toml).map_err(|e| anyhow::anyhow!("{e}"))?;
		let mut assignment = Assignment {
			items: config.items.into_iter().map(GradingItem::from).collect(),
			..Assignment::named("hw")
		};
		settle(&mut assignment, &config.grading, specs)
	}

	#[test]
	fn test_grading_defaults_agree() {
		let omitted = parse("").unwrap().grading;
		let empty = parse("[grading]").unwrap().grading;
		assert_eq!(omitted, GradingConfig::default());
		assert_eq!(empty, GradingConfig::default());
		assert_eq!(omitted.scale, 100.0);
		assert_eq!(omitted.missing_file, crate::models::MissingPolicy::Withheld);
	}

	#[test]
	fn test_declared_items_carry_points_and_aggregation() {
		let config = parse(
			"[[items]]\nid = \"a\"\npoints = 3\naggregation = \"all_or_nothing\"\n\
			 [grading]\ncurve = { kind = \"template\", name = \"sqrt\", lower = 60, upper = 100 }\n",
		)
		.unwrap();
		let item = GradingItem::from(config.items[0].clone());
		assert_eq!(item.points, 3);
		assert_eq!(item.aggregation, crate::models::Aggregation::AllOrNothing);
	}

	#[test]
	fn test_typos_and_missing_aggregation_are_refused() {
		for toml in [
			"[grading]\nmissing = \"witheld\"\n",
			"[grading]\npionts = 1\n",
			"[[items]]\nid = \"a\"\npionts = 2\naggregation = \"proportional\"\n",
			"[[items]]\nid = \"a\"\npoints = 2\n",
			"[grading]\ncurve = { kind = \"template\", name = \"cubic\", lower = 0, upper = 1 }\n",
		] {
			assert!(parse(toml).is_err(), "{toml}");
		}
	}

	#[test]
	fn test_settle_derives_items_when_none_are_declared() {
		let mut assignment = Assignment::named("hw");
		let policy = settle(
			&mut assignment,
			&GradingConfig::default(),
			&[spec("a", false), spec("b", false)],
		)
		.unwrap();
		assert!(policy.derived_items());
		assert_eq!(
			assignment.items,
			vec![GradingItem::new("a"), GradingItem::new("b")]
		);
		assert!(items_toml(&assignment.items).contains("id = \"b\"\npoints = 1"));
	}

	#[test]
	fn test_settle_refuses_everything_wrong_at_once() {
		let item = |id: &str, points: u32| {
			format!("[[items]]\nid = \"{id}\"\npoints = {points}\naggregation = \"proportional\"\n")
		};
		let cases = [
			(
				format!("{}{}", item("a", 1), item("a", 1)),
				"declared twice",
			),
			(
				format!("{}{}", item("a", 1), item("ghost", 1)),
				"has no test spec",
			),
			(item("zero", 0), "0 points"),
			(
				item("a", 1) + "[grading]\nlint_points = 1\n",
				"no test spec declares [lint]",
			),
			(item("a", 1) + "[grading]\nscale = 0\n", "scale"),
			(
				item("a", 1) + "[grading]\ncurve = { kind = \"formula\", formula = \"nope\" }\n",
				"does not compile",
			),
		];
		for (toml, expected) in cases {
			let specs = [spec("a", false), spec("zero", false)];
			let specs = if toml.contains("\"zero\"") {
				&specs[1..]
			} else {
				&specs[..1]
			};
			let err = settle_toml(&toml, specs).err().expect(expected).to_string();
			assert!(err.contains(expected), "{toml}\n→ {err}");
		}
		let err = settle_toml(&item("a", 1), &[spec("a", false), spec("b", false)])
			.err()
			.unwrap()
			.to_string();
		assert!(err.contains("'b' is not a declared item"), "{err}");
		let err = settle_toml("", &[spec("a", false), spec("a", false)])
			.err()
			.unwrap()
			.to_string();
		assert!(err.contains("two test specs are named 'a'"), "{err}");
		assert!(
			settle_toml(
				&(item("a", 1) + "[grading]\nlint_points = 1\n"),
				&[spec("a", true)]
			)
			.is_ok()
		);
	}
}
