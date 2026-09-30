use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table, presets::UTF8_FULL};
use owo_colors::OwoColorize;
use scriptmark::export;
use scriptmark::models::{Fault, GradeOutcome, ItemOutcome, StudentReport, TestStatus};

/// Display a summary table of all student results.
pub fn display_summary(reports: &[&StudentReport], title: &str) {
	let mut table = Table::new();
	table
		.load_preset(UTF8_FULL)
		.set_content_arrangement(ContentArrangement::Dynamic)
		.set_header(vec![
			Cell::new("Student").fg(Color::White),
			Cell::new("ID").fg(Color::Cyan),
			Cell::new("State").fg(Color::Magenta),
			Cell::new("Reason").fg(Color::Magenta),
			Cell::new("Score").fg(Color::Yellow),
			Cell::new("Raw").fg(Color::Yellow),
			Cell::new("Grade").fg(Color::Yellow),
			Cell::new("Cases").fg(Color::White),
		]);

	for report in reports {
		let right = |text: String| Cell::new(text).set_alignment(CellAlignment::Right);
		let (state, reason, score, raw, final_grade) = match &report.grade {
			None => (
				Cell::new("UNSCORED").fg(Color::DarkGrey),
				String::new(),
				"-".into(),
				"-".into(),
				right("-".into()),
			),
			Some(grade) => {
				let reason = grade.reason().map(|r| export::word(&r)).unwrap_or_default();
				match grade.outcome {
					GradeOutcome::Graded {
						score,
						raw_grade,
						final_grade,
						..
					} => (
						Cell::new("GRADED").fg(Color::Green),
						reason,
						format!(
							"{}/{}",
							export::number(score, export::POINTS_DECIMALS),
							export::number(grade.max, export::POINTS_DECIMALS)
						),
						export::number(raw_grade, grade.basis.decimals),
						right(export::number(final_grade, grade.basis.decimals))
							.fg(grade_color(final_grade / grade.basis.scale)),
					),
					GradeOutcome::Withheld { .. } => (
						Cell::new("WITHHELD").fg(Color::Magenta),
						reason,
						"-".into(),
						"-".into(),
						right("-".into()),
					),
				}
			}
		};

		table.add_row(vec![
			Cell::new(report.student_name.as_deref().unwrap_or("N/A")),
			Cell::new(&report.student_id).fg(Color::Cyan),
			state,
			Cell::new(reason),
			right(score),
			right(raw),
			final_grade,
			right(format!(
				"{}/{}",
				report.total_passed(),
				report.total_cases()
			)),
		]);
	}

	println!(
		"\n{}",
		format!(" Grading Summary — {title} ")
			.bold()
			.on_blue()
			.white()
	);
	println!("{table}");
}

/// Green, yellow, red by the share of the scale a grade reached.
fn grade_color(fraction: f64) -> Color {
	if fraction >= 0.9 {
		Color::Green
	} else if fraction >= 0.6 {
		Color::Yellow
	} else {
		Color::Red
	}
}

/// Display detailed failure reports for students with failures.
pub fn display_failures(reports: &[&StudentReport]) {
	let failed: Vec<_> = reports
		.iter()
		.filter(|r| {
			r.error.is_some()
				|| r.grade.as_ref().is_some_and(|g| g.is_withheld())
				|| r.status() == TestStatus::Failed
				|| r.status() == TestStatus::Error
		})
		.collect();

	if failed.is_empty() {
		return;
	}

	println!("\n{}", " Failure Details ".bold().on_red().white());

	for report in failed {
		println!(
			"\n{} {}",
			"Student:".dimmed(),
			format!(
				"{} ({})",
				report.student_name.as_deref().unwrap_or("N/A"),
				report.student_id
			)
			.bold()
		);

		if let Some(error) = &report.error {
			println!("  {} {error}", "ERROR".red().bold());
		}
		if let Some(grade) = &report.grade
			&& let GradeOutcome::Withheld { reason, detail } = &grade.outcome
		{
			let blocking = grade.items.iter().find_map(|item| match &item.outcome {
				ItemOutcome::Withheld {
					blocking_case: Some(case),
					..
				} => Some(format!(" (item '{}', case '{case}')", item.item_id)),
				_ => None,
			});
			println!(
				"  {} {}{}{}",
				"NO GRADE".magenta().bold(),
				export::word(reason),
				blocking.unwrap_or_default(),
				detail
					.as_deref()
					.map(|d| format!(": {d}"))
					.unwrap_or_default(),
			);
		}

		for test_result in &report.test_results {
			for case in &test_result.cases {
				if case.status == TestStatus::Passed {
					continue;
				}

				let status_str = match case.status {
					TestStatus::Failed => "FAIL".red().to_string(),
					TestStatus::Error => "ERROR".red().bold().to_string(),
					TestStatus::Timeout => "TIMEOUT".yellow().to_string(),
					TestStatus::Missing => "MISSING".dimmed().to_string(),
					TestStatus::Passed => continue,
				};
				// A teacher's or the machine's failure is flagged: the student cannot fix it.
				let cause = case
					.cause
					.and_then(|c| serde_json::to_value(c).ok())
					.and_then(|v| v.as_str().map(str::to_string));
				let owner = match (case.fault, cause) {
					(Some(Fault::Teacher), cause) => {
						format!(
							" {}",
							format!("(teacher: {})", cause.unwrap_or_default())
								.magenta()
								.bold()
						)
					}
					(Some(Fault::Environment), cause) => format!(
						" {}",
						format!("(environment: {})", cause.unwrap_or_default())
							.magenta()
							.bold()
					),
					(_, Some(cause)) if cause != "wrong" => {
						format!(" {}", format!("({cause})").dimmed())
					}
					_ => String::new(),
				};

				println!(
					"  {}{} [{}] {}",
					status_str,
					owner,
					test_result.item_id.dimmed(),
					case.case_name
				);

				if let Some(failure) = &case.failure {
					println!("    {}", failure.message.dimmed());
				}

				if let (Some(expected), Some(actual)) = (&case.expected, &case.actual) {
					println!(
						"    {} {} {} {}",
						"expected:".dimmed(),
						expected.green(),
						"got:".dimmed(),
						actual.red()
					);
				}
			}
		}
	}
}

/// Print a one-line status summary: who has a grade, and why the rest do not.
pub fn display_stats(reports: &[&StudentReport]) {
	let graded: Vec<f64> = reports.iter().filter_map(|r| r.final_grade()).collect();
	let decimals = reports
		.iter()
		.find_map(|r| r.grade.as_ref())
		.map_or(2, |g| g.basis.decimals);
	let zeros = graded.iter().filter(|g| **g == 0.0).count();
	let mut withheld = std::collections::BTreeMap::<String, usize>::new();
	for grade in reports.iter().filter_map(|r| r.grade.as_ref()) {
		if let GradeOutcome::Withheld { reason, .. } = grade.outcome {
			*withheld.entry(export::word(&reason)).or_default() += 1;
		}
	}
	let unscored = reports.iter().filter(|r| r.grade.is_none()).count();

	let average = if graded.is_empty() {
		"-".to_string()
	} else {
		export::number(graded.iter().sum::<f64>() / graded.len() as f64, decimals)
	};
	println!(
		"\n{} {} students: {} graded ({} zero, average {}), {} withheld",
		"Summary:".bold(),
		reports.len(),
		graded.len().to_string().green(),
		zeros,
		average,
		withheld.values().sum::<usize>().to_string().magenta(),
	);
	for (why, n) in &withheld {
		println!("  {n:>4} withheld: {why}");
	}
	if unscored > 0 {
		println!("  {unscored:>4} not scored");
	}
}
