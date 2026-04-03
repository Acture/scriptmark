use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table, presets::UTF8_FULL};
use owo_colors::OwoColorize;
use scriptmark::models::{StudentReport, TestStatus};

/// Display a summary table of all student results.
pub fn display_summary(reports: &[&StudentReport], title: &str) {
	let mut table = Table::new();
	table
		.load_preset(UTF8_FULL)
		.set_content_arrangement(ContentArrangement::Dynamic)
		.set_header(vec![
			Cell::new("Student").fg(Color::White),
			Cell::new("ID").fg(Color::Cyan),
			Cell::new("Status").fg(Color::Magenta),
			Cell::new("Passed").fg(Color::Green),
			Cell::new("Failed").fg(Color::Red),
			Cell::new("Total").fg(Color::White),
			Cell::new("Pass Rate").fg(Color::Yellow),
			Cell::new("Grade").fg(Color::Yellow),
		]);

	for report in reports {
		let status = report.status();
		let (status_str, status_color) = match status {
			TestStatus::Passed => ("PASSED", Color::Green),
			TestStatus::Failed => ("FAILED", Color::Red),
			TestStatus::Missing => ("MISSING", Color::DarkGrey),
			TestStatus::Error => ("ERROR", Color::Red),
			TestStatus::Timeout => ("TIMEOUT", Color::Yellow),
		};

		let grade_str = report
			.final_grade
			.map(|g| format!("{g:.1}"))
			.unwrap_or_else(|| "-".to_string());

		table.add_row(vec![
			Cell::new(report.student_name.as_deref().unwrap_or("N/A")),
			Cell::new(&report.student_id).fg(Color::Cyan),
			Cell::new(status_str).fg(status_color),
			Cell::new(report.total_passed()).set_alignment(CellAlignment::Right),
			Cell::new(report.total_failed()).set_alignment(CellAlignment::Right),
			Cell::new(report.total_cases()).set_alignment(CellAlignment::Right),
			Cell::new(format!("{:.1}%", report.pass_rate())).set_alignment(CellAlignment::Right),
			Cell::new(&grade_str).set_alignment(CellAlignment::Right),
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

/// Display detailed failure reports for students with failures.
pub fn display_failures(reports: &[&StudentReport]) {
	let failed: Vec<_> = reports
		.iter()
		.filter(|r| r.status() == TestStatus::Failed || r.status() == TestStatus::Error)
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

		for test_result in &report.test_results {
			for case in &test_result.cases {
				if case.status == TestStatus::Passed {
					continue;
				}

				let status_str = match case.status {
					TestStatus::Failed => "FAIL".red().to_string(),
					TestStatus::Error => "ERROR".red().bold().to_string(),
					TestStatus::Timeout => "TIMEOUT".yellow().to_string(),
					_ => continue,
				};

				println!(
					"  {} [{}] {}",
					status_str,
					test_result.spec_name.dimmed(),
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

/// Print a one-line status summary.
pub fn display_stats(reports: &[&StudentReport]) {
	let total_students = reports.len();
	let passed = reports
		.iter()
		.filter(|r| r.status() == TestStatus::Passed)
		.count();
	let failed = total_students - passed;

	let total_cases: usize = reports.iter().map(|r| r.total_cases()).sum();
	let total_passed: usize = reports.iter().map(|r| r.total_passed()).sum();

	println!(
		"\n{} {} students ({} passed, {} failed), {} test cases ({} passed)",
		"Summary:".bold(),
		total_students,
		passed.to_string().green(),
		failed.to_string().red(),
		total_cases,
		total_passed.to_string().green(),
	);
}
