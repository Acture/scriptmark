use std::path::Path;

use anyhow::Result;
use scriptmark::models::StudentReport;
use scriptmark::similarity::SimilarityPair;

/// Generate a self-contained HTML report from grading results.
///
/// The HTML file contains inline CSS + JS with the data embedded as JSON.
/// Open in any browser — no server needed.
///
/// Security note: This is a locally-generated report from trusted grading data,
/// not a user-facing web application. innerHTML usage is intentional for template
/// rendering of our own data.
pub fn generate_html_report(
	reports: &[StudentReport],
	similarity: Option<&[SimilarityPair]>,
	title: &str,
	output_path: &Path,
) -> Result<()> {
	let reports_json = serde_json::to_string(reports)?;
	let similarity_json = similarity
		.map(|s| {
			serde_json::to_string(
				&s.iter()
					.map(|p| {
						serde_json::json!({
							"a": p.student_a,
							"b": p.student_b,
							"score": p.score,
							"style": p.style_score,
							"structure": p.structure_score,
						})
					})
					.collect::<Vec<_>>(),
			)
			.unwrap_or_else(|_| "[]".to_string())
		})
		.unwrap_or_else(|| "[]".to_string());

	let html = include_str!("report_template.html")
		.replace("{{TITLE}}", title)
		.replace("{{REPORTS_JSON}}", &reports_json)
		.replace("{{SIMILARITY_JSON}}", &similarity_json);

	if let Some(parent) = output_path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(output_path, html)?;
	Ok(())
}
