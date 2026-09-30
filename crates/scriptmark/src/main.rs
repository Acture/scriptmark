mod display;
mod report;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use scriptmark::assignment::{self, Declared};
use scriptmark::discovery::{LocalInputOptions, load_local_input};
use scriptmark::grading::{self, Policy};
use scriptmark::models::{
	AssignmentInput, DiagnosticSeverity, StudentKey, StudentReport, StudentSubmission,
	SubmissionOutcome, TestSpec,
};
use scriptmark::roster::load_roster;
use scriptmark::runner::frozen::{self, Frozen, Generation};
use scriptmark::runner::generation::SeedSource;
use scriptmark::runner::orchestrator::{self, RunOptions};
use scriptmark::runner::prepare::prepare;
use scriptmark::runner::python::PythonExecutor;
use scriptmark::spec_loader::load_specs_from_dir;

#[derive(Parser)]
#[command(
	name = "scriptmark",
	about = "Automated grading CLI for student assignments"
)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// Run tests + summarize + display — all in one step
	Grade(GradeArgs),
	/// Run tests only, output raw results to JSON
	Run(RunArgs),
	/// Summarize existing results (re-analyze without re-running)
	Summarize(SummarizeArgs),
	/// Canvas LMS: browse courses, and fetch an assignment for grading
	#[command(subcommand)]
	Canvas(CanvasCommand),
	/// Pull student roster from Canvas LMS
	RosterPull(RosterPullArgs),
	/// Push grades to Canvas LMS
	GradesPush(GradesPushArgs),
	/// Detect code similarity between student submissions
	Similarity(SimilarityArgs),
	/// Generate an HTML report from grading results
	Report(ReportArgs),
	/// Launch interactive TUI
	Tui {
		/// Database file path
		#[arg(long, default_value = "scriptmark.db")]
		db: PathBuf,
	},
	/// Database management commands
	Db(DbCommand),
}

/// Where a batch's generated inputs come from. They are frozen beside `--output`, as
/// `<stem>.cases.json`, once the run is done.
#[derive(clap::Args)]
struct FrozenArgs {
	/// Grade on the inputs frozen in FILE instead of generating them
	#[arg(long, value_name = "FILE")]
	replay: Option<PathBuf>,

	/// Generate new inputs, even though the ones frozen beside --output differ
	#[arg(long, conflicts_with = "replay")]
	fresh: bool,
}

#[derive(Parser)]
struct GradeArgs {
	/// Directories containing student submissions
	#[arg(required_unless_present = "canvas", conflicts_with = "canvas")]
	submissions: Vec<PathBuf>,

	/// Grade from a Canvas bundle written by `scriptmark canvas fetch`.
	///
	/// Offline: re-grading does not re-download a class's work.
	#[arg(long)]
	canvas: Option<PathBuf>,

	/// Directory containing TOML test spec files
	#[arg(short = 't', long = "tests")]
	tests_dir: PathBuf,

	/// Output file for raw results (JSON)
	#[arg(short, long, default_value = "output/results.json")]
	output: PathBuf,

	/// Path to roster CSV (name,_,student_id)
	#[arg(short, long)]
	roster: Option<PathBuf>,

	/// Path to assignment.toml. Defaults to one beside the tests directory.
	#[arg(long)]
	assignment: Option<PathBuf>,

	/// Seconds each call may run: loading the student's file, each setup call, each case
	/// and step, each checker
	#[arg(long, default_value = "10", value_parser = clap::value_parser!(u64).range(1..=86_400))]
	timeout: u64,

	/// Units (cases or scenarios) running at once; defaults to the number of CPUs
	#[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
	concurrency: Option<u64>,

	/// Python interpreter command
	#[arg(long, default_value = "python3")]
	python: String,

	/// Archive results to this directory (JSON/CSV)
	#[arg(short, long)]
	archive: Option<PathBuf>,

	/// Archive format
	#[arg(short, long, default_value = "csv")]
	format: String,

	/// Save results to SQLite database
	#[arg(long)]
	db: Option<PathBuf>,

	#[command(flatten)]
	frozen: FrozenArgs,
}

#[derive(Parser)]
struct RunArgs {
	/// Directories containing student submissions
	#[arg(required_unless_present = "canvas", conflicts_with = "canvas")]
	submissions: Vec<PathBuf>,

	/// Grade from a Canvas bundle written by `scriptmark canvas fetch`.
	///
	/// Offline: re-grading does not re-download a class's work.
	#[arg(long)]
	canvas: Option<PathBuf>,

	/// Directory containing TOML test spec files
	#[arg(short = 't', long = "tests")]
	tests_dir: PathBuf,

	/// Output file for raw results (JSON)
	#[arg(short, long, default_value = "output/results.json")]
	output: PathBuf,

	/// Path to roster CSV (name,_,student_id)
	#[arg(short, long)]
	roster: Option<PathBuf>,

	/// Path to assignment.toml. Defaults to one beside the tests directory.
	#[arg(long)]
	assignment: Option<PathBuf>,

	/// Seconds each call may run: loading the student's file, each setup call, each case
	/// and step, each checker
	#[arg(long, default_value = "10", value_parser = clap::value_parser!(u64).range(1..=86_400))]
	timeout: u64,

	/// Units (cases or scenarios) running at once; defaults to the number of CPUs
	#[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
	concurrency: Option<u64>,

	/// Python interpreter command
	#[arg(long, default_value = "python3")]
	python: String,

	#[command(flatten)]
	frozen: FrozenArgs,
}

#[derive(Parser)]
struct SummarizeArgs {
	/// Path to results JSON file, as `grade` wrote it
	results: PathBuf,

	/// Path to roster CSV
	#[arg(short, long)]
	roster: Option<PathBuf>,
}

#[derive(Subcommand)]
enum CanvasCommand {
	/// List the courses this token can see
	Courses(CanvasListArgs),
	/// List a course's assignments
	Assignments(CanvasAssignmentsArgs),
	/// Fetch an assignment's roster, submissions and attachments into a bundle
	Fetch(CanvasFetchArgs),
}

#[derive(Parser)]
struct CanvasListArgs {
	/// Canvas API base URL. Defaults to $CANVAS_URL.
	#[arg(long, env = "CANVAS_URL")]
	canvas_url: String,
}

#[derive(Parser)]
struct CanvasAssignmentsArgs {
	#[arg(long, env = "CANVAS_URL")]
	canvas_url: String,

	#[arg(long)]
	course_id: u64,
}

#[derive(Parser)]
struct CanvasFetchArgs {
	#[arg(long, env = "CANVAS_URL")]
	canvas_url: String,

	/// Canvas course id. Taken from --assignment's toml when omitted.
	#[arg(long)]
	course_id: Option<u64>,

	/// Canvas assignment id. Taken from --assignment's toml when omitted.
	#[arg(long)]
	assignment_id: Option<u64>,

	/// An assignment.toml supplying the course and assignment ids.
	///
	/// Named explicitly rather than searched for: the implicit search resolves relative to
	/// a tests directory, and fetching has none.
	#[arg(long)]
	assignment: Option<PathBuf>,

	/// Where to write the bundle
	#[arg(short, long)]
	output: PathBuf,

	/// How many attachments to download at once.
	///
	/// One by default. Canvas throttles on a per-token cost bucket with no published rate,
	/// so raising this is the teacher's call about their own instance.
	#[arg(long, default_value = "1")]
	download_concurrency: usize,
}

#[derive(Parser)]
struct RosterPullArgs {
	/// Canvas API base URL (e.g. https://canvas.university.edu)
	#[arg(long)]
	canvas_url: String,

	/// Canvas course ID
	#[arg(long)]
	course_id: u64,

	/// Output roster CSV path
	#[arg(short, long, default_value = "roster.csv")]
	output: PathBuf,
}

#[derive(Parser)]
struct GradesPushArgs {
	/// Canvas API base URL
	#[arg(long)]
	canvas_url: String,

	/// Canvas course ID
	#[arg(long)]
	course_id: u64,

	/// Canvas assignment ID
	#[arg(long)]
	assignment_id: u64,

	/// Path to results JSON file (from scriptmark grade)
	results: PathBuf,
}

#[derive(Parser)]
struct SimilarityArgs {
	/// Directories containing student submissions
	#[arg(required = true)]
	submissions: Vec<PathBuf>,

	/// N-gram size for fingerprinting (default: 25)
	#[arg(long, default_value = "25")]
	ngram_size: usize,

	/// Minimum similarity threshold to report (0.0-1.0, default: 0.6)
	#[arg(long, default_value = "0.6")]
	threshold: f64,

	/// Output CSV file for similarity report
	#[arg(short, long)]
	output: Option<PathBuf>,
}

#[derive(Parser)]
struct ReportArgs {
	/// Path to results JSON file (from scriptmark grade)
	results: PathBuf,

	/// Output HTML report path
	#[arg(short, long, default_value = "report.html")]
	output: PathBuf,

	/// Title for the report
	#[arg(long, default_value = "Grading Report")]
	title: String,

	/// Also include similarity data (provide submissions dir)
	#[arg(long)]
	similarity_dir: Option<PathBuf>,

	/// Similarity threshold
	#[arg(long, default_value = "0.6")]
	similarity_threshold: f64,
}

#[derive(Parser)]
struct DbCommand {
	#[command(subcommand)]
	action: DbAction,
}

#[derive(Subcommand)]
enum DbAction {
	/// Initialize a new database
	Init {
		/// Database file path
		#[arg(default_value = "scriptmark.db")]
		path: PathBuf,
	},
	/// Import a roster CSV into the database
	ImportRoster {
		/// Roster CSV file
		roster: PathBuf,
		/// Database file path
		#[arg(long, default_value = "scriptmark.db")]
		db: PathBuf,
	},
	/// List all grading sessions
	Sessions {
		/// Database file path
		#[arg(long, default_value = "scriptmark.db")]
		db: PathBuf,
	},
	/// Show a student's history across all sessions
	History {
		/// Student ID
		student_id: String,
		/// Database file path
		#[arg(long, default_value = "scriptmark.db")]
		db: PathBuf,
	},
}

/// Build the unified input from local directories.
fn build_local_input(
	submissions: &[PathBuf],
	declared: &Declared,
	roster_path: Option<&PathBuf>,
) -> Result<AssignmentInput> {
	let (assignment, attempt_policy) = (declared.assignment.clone(), declared.attempt_policy);

	let roster = match roster_path {
		Some(path) => Some(load_roster(path).context("Failed to load roster")?),
		None => None,
	};

	let input = load_local_input(
		submissions,
		LocalInputOptions {
			assignment,
			roster: roster.as_ref(),
			attempt_policy,
		},
	)
	.context("Failed to discover submissions")?;

	report_input(&input);

	// An Error diagnostic means the input cannot be trusted — a roster that disagrees with
	// itself about who a 学号 belongs to would attribute somebody's work to the wrong name.
	// Stop before running anything rather than producing results nobody should act on.
	let errors: Vec<String> = input.errors().map(|d| d.to_string()).collect();
	if !errors.is_empty() {
		anyhow::bail!(
			"refusing to grade: {} problem(s) with the input\n  {}",
			errors.len(),
			errors.join("\n  ")
		);
	}

	Ok(input)
}

/// Print the import summary and every anomaly the adapters recorded. Adapters never print
/// themselves — this is the only place diagnostics reach a terminal.
fn report_input(input: &AssignmentInput) {
	use owo_colors::OwoColorize;

	let counts = [
		(SubmissionOutcome::Executable, "executable"),
		(SubmissionOutcome::SubmittedEmpty, "submitted but empty"),
		(SubmissionOutcome::ReceivedUnmatched, "received, unmatched"),
		(SubmissionOutcome::NotSubmitted, "not submitted"),
	];
	println!("Found {} students:", input.student_count());
	for (outcome, label) in counts {
		let n = input.with_outcome(outcome).count();
		if n > 0 {
			println!("  {n:>4}  {label}");
		}
	}
	if !input.unmatched.is_empty() {
		println!(
			"  {:>4}  files with no identifiable owner",
			input.unmatched.len()
		);
	}

	// 免交 is not 缺交, and the difference matters to whoever reads this.
	let excused = input.students.iter().filter(|s| s.is_excused()).count();
	if excused > 0 {
		println!("  {excused:>4}  excused by the teacher");
	}

	// 明确使用哪次提交: say so wherever a student is not being graded on their first try.
	let later: Vec<&scriptmark::models::StudentSubmission> = input
		.students
		.iter()
		.filter(|s| s.selected_attempt().is_some_and(|a| a.attempt > 1))
		.collect();
	if !later.is_empty() {
		println!(
			"  {:>4}  graded on a later attempt ({} policy)",
			later.len(),
			match input.attempt_policy {
				scriptmark::models::AttemptPolicy::Latest => "latest",
				scriptmark::models::AttemptPolicy::Earliest => "earliest",
			}
		);
		for student in later {
			if let Some(attempt) = student.selected_attempt() {
				println!(
					"        {} -> attempt {}{}",
					student.key(),
					attempt.attempt,
					attempt
						.submitted_at
						.as_deref()
						.map(|t| format!(" ({t})"))
						.unwrap_or_default()
				);
			}
		}
	}

	let errors = input.diagnostics_of(DiagnosticSeverity::Error).count();
	let warnings = input.diagnostics_of(DiagnosticSeverity::Warning).count();
	for diagnostic in &input.diagnostics {
		match diagnostic.severity {
			DiagnosticSeverity::Error => println!("  {} {diagnostic}", "error:".red()),
			DiagnosticSeverity::Warning => println!("  {} {diagnostic}", "warning:".yellow()),
			DiagnosticSeverity::Info => println!("  {} {diagnostic}", "note:".dimmed()),
		}
	}
	if errors + warnings > 0 {
		println!("  ({errors} errors, {warnings} warnings)");
	}
}

#[tokio::main]
async fn main() -> Result<()> {
	let cli = Cli::parse();

	match cli.command {
		Commands::Grade(args) => cmd_grade(args).await,
		Commands::Run(args) => cmd_run(args).await,
		Commands::Summarize(args) => cmd_summarize(args),
		Commands::Canvas(cmd) => cmd_canvas(cmd).await,
		Commands::RosterPull(args) => cmd_roster_pull(args).await,
		Commands::GradesPush(args) => cmd_grades_push(args).await,
		Commands::Similarity(args) => cmd_similarity(args),
		Commands::Report(args) => cmd_report(args),
		Commands::Tui { db } => scriptmark::tui::run_tui(&db).context("TUI error"),
		Commands::Db(cmd) => cmd_db(cmd),
	}
}

/// Read a results file `grade` or `run` wrote. A file from before grades were scored per
/// item is refused rather than reinterpreted: what its numbers meant is not recoverable.
fn parse_results(content: &str) -> Result<Vec<StudentReport>> {
	serde_json::from_str(content).context(
		"failed to parse the results file; results written before per-item grading are not \
		 read — grade the submissions again",
	)
}

/// A fault or cause as the snake_case word the JSON results use; empty when absent.
fn label<T: serde::Serialize>(value: Option<T>) -> String {
	value
		.and_then(|v| serde_json::to_value(v).ok())
		.and_then(|v| v.as_str().map(str::to_string))
		.unwrap_or_default()
}

/// Prepare every test bundle, then run them against every student. A bundle that cannot
/// be prepared stops the run before any student is graded, and so do fresh inputs that
/// would replace other inputs frozen beside `output`.
///
/// Returns the reports and the inputs they were graded on, for `save_frozen` once the
/// results are written.
async fn run_bundles(
	students: &[StudentSubmission],
	specs: Vec<TestSpec>,
	python: &str,
	timeout: u64,
	concurrency: Option<u64>,
	output: &Path,
	options: &FrozenArgs,
) -> Result<(Vec<StudentReport>, Frozen)> {
	let generation = match &options.replay {
		Some(path) => Generation::Replay(Frozen::load(path)?),
		None => Generation::fresh(),
	};
	let executor = Arc::new(PythonExecutor::with_python_cmd(python));
	let bundles = prepare(specs, &generation, executor.clone(), timeout)
		.await
		.context("refusing to grade: the test bundle is not ready")?;
	println!(
		"Prepared {} test bundle(s): {} unit(s) per student",
		bundles.len(),
		bundles.iter().map(|b| b.units.len()).sum::<usize>()
	);
	let inputs = Frozen::of(&bundles);
	let beside = frozen::beside(output);
	if options.replay.is_none() {
		for (spec, templates) in &inputs.specs {
			for (case, made) in templates {
				if let (Some(seed), Some(SeedSource::Drawn)) = (made.seed, made.seed_source) {
					eprintln!(
						"  note: case '{case}' in '{spec}' drew seed {seed}. Write `seed = {seed}` in its [cases.parametrize.random] to keep these inputs, or grade with --replay {}",
						beside.display()
					);
				}
			}
		}
		if !inputs.is_empty() {
			frozen::check_replaceable(&beside, &inputs, options.fresh)
				.map_err(anyhow::Error::msg)
				.context("refusing to grade")?;
		}
	}
	let run_options = RunOptions {
		concurrency: concurrency.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
		python: executor.python_cmd().to_string(),
	};
	// Units run in their own process groups, so the terminal's Ctrl-C reaches only the
	// grader: take them down with it rather than leave them running to their timeouts.
	tokio::select! {
		reports = orchestrator::run_all(students, bundles.into(), executor, &run_options) => Ok((reports, inputs)),
		_ = tokio::signal::ctrl_c() => {
			scriptmark::runner::python::kill_all_units();
			anyhow::bail!("interrupted: every running unit was stopped")
		}
	}
}

/// Write the inputs a batch was graded on beside its results — after them, so an
/// interrupted or failed run replaces neither.
fn save_frozen(inputs: &Frozen, output: &Path) -> Result<()> {
	if inputs.is_empty() {
		return Ok(());
	}
	let path = frozen::beside(output);
	inputs
		.write(&path)
		.with_context(|| format!("failed to write {}", path.display()))?;
	println!("Inputs frozen to {}", path.display());
	Ok(())
}

/// Everything settled before any student runs.
struct Batch {
	input: AssignmentInput,
	specs: Vec<TestSpec>,
	policy: Policy,
}

/// Load the assignment and the specs, settle the items and the policy against each other,
/// then build the input — refusing a bad policy before a single student is run.
fn prepare_batch(
	submissions: &[PathBuf],
	canvas: Option<&PathBuf>,
	tests_dir: &std::path::Path,
	assignment_path: Option<&PathBuf>,
	roster: Option<&PathBuf>,
) -> Result<Batch> {
	let mut declared = assignment::load(assignment_path.map(PathBuf::as_path), tests_dir)?;
	let specs = load_specs_from_dir(tests_dir).context("Failed to load test specifications")?;
	println!("Loaded {} test specs", specs.len());

	let policy = assignment::settle(&mut declared.assignment, &declared.grading, &specs)?;
	if policy.derived_items() {
		eprintln!(
			"  note: no [[items]] declared; each spec is an item worth 1 point. To weight \
			 them, add this to assignment.toml and edit the points:\n\n{}",
			assignment::items_toml(&declared.assignment.items)
		);
	}

	// Names, roster membership and submission state all come from the model, so there is
	// no separate roster merge afterwards.
	let input = match canvas {
		Some(bundle) => build_canvas_input(bundle, &declared)?,
		None => build_local_input(submissions, &declared, roster)?,
	};
	Ok(Batch {
		input,
		specs,
		policy,
	})
}

async fn cmd_grade(args: GradeArgs) -> Result<()> {
	let Batch {
		input,
		specs,
		policy,
	} = prepare_batch(
		&args.submissions,
		args.canvas.as_ref(),
		&args.tests_dir,
		args.assignment.as_ref(),
		args.roster.as_ref(),
	)?;

	let (mut reports, inputs) = run_bundles(
		&input.students,
		specs,
		&args.python,
		args.timeout,
		args.concurrency,
		&args.output,
		&args.frozen,
	)
	.await?;

	let items = &input.assignment.items;
	grading::grade_all(&mut reports, items, &policy)?;
	reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));

	// Display
	let report_refs: Vec<_> = reports.iter().collect();
	display::display_summary(&report_refs, &args.tests_dir.display().to_string());
	display::display_failures(&report_refs);
	display::display_stats(&report_refs);
	for warning in grading::diagnostics(&reports, items) {
		eprintln!("  warning: {warning}");
	}

	// Save raw results
	if let Some(parent) = args.output.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let json = serde_json::to_string_pretty(&reports)?;
	std::fs::write(&args.output, &json)?;
	println!("\nResults saved to {}", args.output.display());
	save_frozen(&inputs, &args.output)?;

	// Archive: the evidence per case, and the grades per student.
	if let Some(archive_dir) = &args.archive {
		std::fs::create_dir_all(archive_dir)?;
		let stem = args
			.tests_dir
			.file_name()
			.and_then(|n| n.to_str())
			.unwrap_or("results");
		let archive_path = archive_dir.join(format!("archive_{stem}.{}", args.format));
		let grades_path = archive_dir.join(format!("grades_{stem}.csv"));
		if !inputs.is_empty() {
			let cases_path = archive_dir.join(format!("cases_{stem}.json"));
			inputs.write(&cases_path)?;
			println!("Inputs written to {}", cases_path.display());
		}
		scriptmark::export::write_grades_csv(
			&reports,
			items,
			std::fs::File::create(&grades_path)?,
		)?;
		println!("Grades written to {}", grades_path.display());

		match args.format.as_str() {
			"json" => {
				std::fs::write(&archive_path, &json)?;
			}
			"csv" => {
				let mut wtr = csv::Writer::from_path(&archive_path)?;
				wtr.write_record([
					"student_name",
					"student_id",
					"submission_state",
					"item_id",
					"case_name",
					"status",
					"actual",
					"expected",
					"message",
					"elapsed_ms",
					"fault",
					"cause",
				])?;
				for report in &reports {
					let state = label(Some(report.submission_state));
					let mut rows = 0usize;
					for test_result in &report.test_results {
						for case in &test_result.cases {
							rows += 1;
							wtr.write_record([
								report.student_name.as_deref().unwrap_or(""),
								&report.student_id,
								&state,
								&test_result.item_id,
								&case.case_name,
								&format!("{:?}", case.status),
								case.actual.as_deref().unwrap_or(""),
								case.expected.as_deref().unwrap_or(""),
								case.failure
									.as_ref()
									.map(|f| f.message.as_str())
									.unwrap_or(""),
								&case.elapsed_ms.map(|ms| ms.to_string()).unwrap_or_default(),
								&label(case.fault),
								&label(case.cause),
							])?;
						}
					}
					// Every student gets at least one row, so the CSV covers the same cohort
					// as the JSON archive rather than quietly dropping non-submitters. Its
					// message says why there is no grade.
					if rows == 0 {
						let why = report.error.clone().unwrap_or_else(|| {
							label(report.grade.as_ref().and_then(|g| g.reason()))
						});
						wtr.write_record([
							report.student_name.as_deref().unwrap_or(""),
							&report.student_id,
							&state,
							"",
							"",
							if report.error.is_some() {
								"Error".to_string()
							} else {
								format!("{:?}", report.status())
							}
							.as_str(),
							"",
							"",
							&why,
							"",
							"",
							"",
						])?;
					}
				}
				wtr.flush()?;
			}
			other => anyhow::bail!("unknown archive format '{other}': use json or csv"),
		}
		println!("Archived to {}", archive_path.display());
	}

	// 9. Save to database if --db specified
	if let Some(db_path) = &args.db {
		let database =
			scriptmark::db::Database::open(db_path).context("Failed to open database")?;

		if let Some(roster) = &input.roster {
			database
				.import_roster(roster)
				.context("Failed to import roster")?;
		}

		let session_id = database
			.save_session(
				&input.assignment.name,
				&reports,
				Some(&serde_json::to_string(&serde_json::json!({
					"grading": policy.config(),
					"items": items,
				}))?),
			)
			.context("Failed to save session to database")?;

		println!(
			"Saved to database: {} (session #{})",
			db_path.display(),
			session_id
		);
	}

	Ok(())
}

async fn cmd_run(args: RunArgs) -> Result<()> {
	// The policy is settled even though nothing is scored: a run whose results cannot be
	// graded should say so now, not after the class has run.
	let Batch { input, specs, .. } = prepare_batch(
		&args.submissions,
		args.canvas.as_ref(),
		&args.tests_dir,
		args.assignment.as_ref(),
		args.roster.as_ref(),
	)?;

	// A JSON array, the same shape `grade` writes; unscored until graded.
	let (results, inputs) = run_bundles(
		&input.students,
		specs,
		&args.python,
		args.timeout,
		args.concurrency,
		&args.output,
		&args.frozen,
	)
	.await?;

	if let Some(parent) = args.output.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let json = serde_json::to_string_pretty(&results)?;
	std::fs::write(&args.output, &json)?;
	println!("Results saved to {}", args.output.display());
	save_frozen(&inputs, &args.output)?;

	Ok(())
}

fn cmd_summarize(args: SummarizeArgs) -> Result<()> {
	let content = std::fs::read_to_string(&args.results).context("Failed to read results file")?;
	let mut reports = parse_results(&content)?;

	if let Some(roster_path) = &args.roster {
		let roster = load_roster(roster_path).context("Failed to load roster")?;
		for report in reports.iter_mut() {
			// `student_id` is a rendered key, so it is parsed back rather than compared as
			// text — otherwise a run made without --roster, whose ids carry a `local:`
			// prefix, would match nothing. `name_of` answers only when the key is
			// unambiguous: with duplicate roster rows there is no single right name, and
			// guessing one would hide the clash.
			if let Some(name) = roster.name_of(&StudentKey::parse(&report.student_id)) {
				report.student_name = Some(name.to_string());
			}
		}
	}

	// Shown as `grade` scored them. Scoring again under another policy is P-678's regrade,
	// which records what changed; a summary that silently re-scored could not.
	reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));

	let report_refs: Vec<_> = reports.iter().collect();
	display::display_summary(&report_refs, &args.results.display().to_string());
	display::display_failures(&report_refs);
	display::display_stats(&report_refs);

	Ok(())
}

/// Build the input for a `--canvas <bundle>` run.
///
/// The declared `assignment.toml` is loaded and handed to `normalize`: it carries the
/// attempt policy, and dropping it would grade every student on their latest attempt while
/// a course that asked for the earliest looked no different. The bundle supplies the Canvas
/// ids only where the toml left them unset, and a genuine disagreement is refused rather
/// than resolved — grading one assignment's submissions against another's declaration is
/// not something a warning covers.
fn build_canvas_input(bundle: &std::path::Path, declared: &Declared) -> Result<AssignmentInput> {
	use scriptmark::canvas::bundle;

	let (assignment, attempt_policy) = (declared.assignment.clone(), declared.attempt_policy);
	let (payload, downloads, diagnostics) = bundle::load(bundle)
		.with_context(|| format!("failed to read the Canvas bundle at {}", bundle.display()))?;

	for (declared, found, what) in [
		(assignment.canvas_course_id, payload.course_id, "course"),
		(
			assignment.canvas_assignment_id,
			payload.assignment_id,
			"assignment",
		),
	] {
		if let (Some(declared), Some(found)) = (declared, found)
			&& declared != found
		{
			anyhow::bail!(
				"refusing to grade: assignment.toml declares Canvas {what} {declared}, but the \
				 bundle was fetched for {what} {found}"
			);
		}
	}

	let input = scriptmark::input::canvas::normalize(
		&payload,
		None,
		&downloads,
		attempt_policy,
		assignment,
	);
	let input = bundle::merge_diagnostics(input, diagnostics);

	report_input(&input);

	let errors: Vec<String> = input.errors().map(|d| d.to_string()).collect();
	if !errors.is_empty() {
		anyhow::bail!(
			"refusing to grade: {} problem(s) with the input\n  {}",
			errors.len(),
			errors.join("\n  ")
		);
	}

	// The record of which attempt was graded, and of every file's provenance, outlives the
	// process that produced it.
	bundle::save_input(bundle, &input)
		.with_context(|| format!("failed to write {}", bundle::input_path(bundle).display()))?;

	Ok(input)
}

async fn cmd_canvas(cmd: CanvasCommand) -> Result<()> {
	use scriptmark::canvas::bundle;
	use std::sync::Arc;

	match cmd {
		CanvasCommand::Courses(args) => {
			let client = scriptmark::canvas::CanvasClient::new(&args.canvas_url)
				.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;
			for course in client.list_courses().await? {
				let term = course
					.term
					.as_ref()
					.and_then(|t| t.name.as_deref())
					.unwrap_or("");
				println!(
					"{:>10}  {}  {}",
					course.id,
					course.name.as_deref().unwrap_or("(unnamed)"),
					term
				);
			}
		}
		CanvasCommand::Assignments(args) => {
			let client = scriptmark::canvas::CanvasClient::new(&args.canvas_url)
				.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;
			for assignment in client.list_assignments(args.course_id).await? {
				println!(
					"{:>10}  {}  {}",
					assignment.id,
					assignment.name.as_deref().unwrap_or("(unnamed)"),
					assignment.due_at.as_deref().unwrap_or("")
				);
			}
		}
		CanvasCommand::Fetch(args) => {
			// The ids come from the flags, or from a toml named explicitly. Nothing is
			// searched for implicitly, and neither source means the run stops rather than
			// guessing at a course.
			let declared = match &args.assignment {
				Some(path) => {
					Some(assignment::load(Some(path), path.parent().unwrap_or(path))?.assignment)
				}
				None => None,
			};
			let course_id = args
				.course_id
				.or_else(|| declared.as_ref().and_then(|a| a.canvas_course_id))
				.context("no --course-id, and no canvas_course_id in --assignment")?;
			let assignment_id = args
				.assignment_id
				.or_else(|| declared.as_ref().and_then(|a| a.canvas_assignment_id))
				.context("no --assignment-id, and no canvas_assignment_id in --assignment")?;

			let client = Arc::new(
				scriptmark::canvas::CanvasClient::new(&args.canvas_url)
					.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?,
			);

			println!(
				"Fetching course {course_id}, assignment {assignment_id} into {}...",
				args.output.display()
			);
			let payload = bundle::fetch(
				client,
				course_id,
				assignment_id,
				&args.output,
				args.download_concurrency,
				|p| {
					let note = if p.skipped { " (already had it)" } else { "" };
					println!("  [{}/{}] {}{note}", p.done, p.total, p.filename);
				},
			)
			.await?;

			println!(
				"Fetched {} enrolled students and {} submission rows.",
				payload.users.len(),
				payload.submissions.len()
			);
			println!(
				"Grade it with: scriptmark grade --canvas {} -t tests/",
				args.output.display()
			);
		}
	}
	Ok(())
}

async fn cmd_roster_pull(args: RosterPullArgs) -> Result<()> {
	let client = scriptmark::canvas::CanvasClient::new(&args.canvas_url)
		.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;

	println!("Pulling roster from Canvas course {}...", args.course_id);
	let roster = client
		.pull_roster(args.course_id)
		.await
		.context("Failed to pull roster from Canvas")?;

	println!("Found {} students", roster.len());

	scriptmark::canvas::CanvasClient::save_roster_csv(&roster, &args.output)
		.context("Failed to save roster CSV")?;

	println!("Roster saved to {}", args.output.display());
	Ok(())
}

async fn cmd_grades_push(args: GradesPushArgs) -> Result<()> {
	let client = scriptmark::canvas::CanvasClient::new(&args.canvas_url)
		.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;

	let content = std::fs::read_to_string(&args.results).context("Failed to read results file")?;
	let reports = parse_results(&content)?;

	// Only graded students are pushed — a real 0 included, a withheld grade never. The
	// Canvas user id is the one import recorded: parsing student_id as an integer would
	// either fail for every 学号 or, worse, post to whichever user held that number.
	let scriptmark::export::PushSet { grades, skipped } =
		scriptmark::export::grades_to_push(&reports)?;
	for (why, n) in &skipped {
		println!("Skipping {n} student(s): {why}");
	}

	println!(
		"Pushing {} grades to Canvas assignment {}...",
		grades.len(),
		args.assignment_id
	);
	let results = client
		.push_grades(args.course_id, args.assignment_id, &grades)
		.await
		.context("Failed to push grades to Canvas")?;

	println!("Successfully pushed {} grades", results.len());
	Ok(())
}

fn cmd_similarity(args: SimilarityArgs) -> Result<()> {
	use scriptmark::similarity::compare_submissions;

	// Collect files grouped by student ID
	let mut submissions: std::collections::HashMap<String, Vec<PathBuf>> =
		std::collections::HashMap::new();

	for dir in &args.submissions {
		for entry in std::fs::read_dir(dir)? {
			let entry = entry?;
			let path = entry.path();
			if path.extension().and_then(|e| e.to_str()) != Some("py") {
				continue;
			}
			let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
			let sid = filename.split('_').next().unwrap_or("").to_string();
			if !sid.is_empty() {
				submissions.entry(sid).or_default().push(path);
			}
		}
	}

	println!(
		"Comparing {} students (n-gram={}, threshold={:.0}%)",
		submissions.len(),
		args.ngram_size,
		args.threshold * 100.0
	);

	let pairs = compare_submissions(&submissions, args.ngram_size, args.threshold);

	if pairs.is_empty() {
		println!(
			"No pairs above {:.0}% similarity threshold.",
			args.threshold * 100.0
		);
		return Ok(());
	}

	use owo_colors::OwoColorize;

	println!(
		"\n{} {} pairs above threshold:\n",
		"Found".bold(),
		pairs.len()
	);

	for pair in &pairs {
		let color = if pair.score > 0.9 {
			"\x1b[31m" // red
		} else if pair.score > 0.75 {
			"\x1b[33m" // yellow
		} else {
			"\x1b[32m" // green
		};
		println!(
			"  {}{:.1}%\x1b[0m (style:{:.0}% struct:{:.0}%)  {} ↔ {}",
			color,
			pair.score * 100.0,
			pair.style_score * 100.0,
			pair.structure_score * 100.0,
			pair.student_a,
			pair.student_b,
		);
	}

	if let Some(output) = &args.output {
		let mut wtr = csv::Writer::from_path(output)?;
		wtr.write_record(["student_a", "student_b", "combined", "style", "structure"])?;
		for pair in &pairs {
			wtr.write_record([
				&pair.student_a,
				&pair.student_b,
				&format!("{:.4}", pair.score),
				&format!("{:.4}", pair.style_score),
				&format!("{:.4}", pair.structure_score),
			])?;
		}
		wtr.flush()?;
		println!("\nReport saved to {}", output.display());
	}

	Ok(())
}

fn cmd_report(args: ReportArgs) -> Result<()> {
	let content = std::fs::read_to_string(&args.results).context("Failed to read results file")?;
	let reports = parse_results(&content)?;

	let similarity = if let Some(sim_dir) = &args.similarity_dir {
		let mut submissions: std::collections::HashMap<String, Vec<PathBuf>> =
			std::collections::HashMap::new();
		for entry in std::fs::read_dir(sim_dir)? {
			let entry = entry?;
			let path = entry.path();
			if path.extension().and_then(|e| e.to_str()) != Some("py") {
				continue;
			}
			let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
			let sid = filename.split('_').next().unwrap_or("").to_string();
			if !sid.is_empty() {
				submissions.entry(sid).or_default().push(path);
			}
		}
		Some(scriptmark::similarity::compare_submissions(
			&submissions,
			25,
			args.similarity_threshold,
		))
	} else {
		None
	};

	report::generate_html_report(&reports, similarity.as_deref(), &args.title, &args.output)?;

	println!("HTML report generated: {}", args.output.display());
	Ok(())
}

fn cmd_db(cmd: DbCommand) -> Result<()> {
	match cmd.action {
		DbAction::Init { path } => {
			let _db =
				scriptmark::db::Database::open(&path).context("Failed to initialize database")?;
			println!("Database initialized: {}", path.display());
			Ok(())
		}
		DbAction::ImportRoster { roster, db } => {
			let database =
				scriptmark::db::Database::open(&db).context("Failed to open database")?;
			let roster_csv =
				scriptmark::roster::load_roster(&roster).context("Failed to load roster CSV")?;
			for diagnostic in &roster_csv.diagnostics {
				println!("  warning: {diagnostic}");
			}
			let count = database
				.import_roster(&roster_csv)
				.context("Failed to import roster")?;
			println!("Imported {} students into {}", count, db.display());
			Ok(())
		}
		DbAction::Sessions { db } => {
			let database =
				scriptmark::db::Database::open(&db).context("Failed to open database")?;
			let sessions = database
				.list_sessions()
				.context("Failed to list sessions")?;
			if sessions.is_empty() {
				println!("No sessions found.");
				return Ok(());
			}
			use owo_colors::OwoColorize;
			println!(
				"{:>4}  {:<20}  {:>8}  {:>8}  Date",
				"ID", "Assignment", "Students", "Avg"
			);
			println!("{}", "-".repeat(70));
			for s in &sessions {
				println!(
					"{:>4}  {:<20}  {:>8}  {:>7}  {}",
					s.id.to_string().cyan(),
					s.assignment,
					s.student_count,
					s.avg_grade
						.map(|a| format!("{a:.1}"))
						.unwrap_or_else(|| "-".into()),
					s.created_at.dimmed(),
				);
			}
			Ok(())
		}
		DbAction::History { student_id, db } => {
			let database =
				scriptmark::db::Database::open(&db).context("Failed to open database")?;
			let name = database.get_student_name(&student_id);
			let history = database
				.get_student_history(&student_id)
				.context("Failed to get student history")?;
			if history.is_empty() {
				println!("No history found for student '{}'.", student_id);
				return Ok(());
			}
			use owo_colors::OwoColorize;
			println!("History for {} ({}):\n", name.bold(), student_id.cyan());
			println!(
				"{:<15}  {:>24}  {:>10}  {:>8}/{:<8}  Date",
				"Assignment", "Grade", "Pass Rate", "Passed", "Total"
			);
			println!("{}", "-".repeat(90));
			for (session, result) in &history {
				let grade_color = match result.fraction() {
					Some(f) if f >= 0.9 => "\x1b[32m",
					Some(f) if f >= 0.7 => "\x1b[34m",
					Some(_) => "\x1b[31m",
					None => "\x1b[2m",
				};
				let grade_text = result.grade_text();
				println!(
					"{:<15}  {}{:>24}\x1b[0m  {:>9.1}%  {:>8}/{}  {}",
					session.assignment,
					grade_color,
					grade_text,
					result.pass_rate,
					result.passed_cases,
					result.total_cases,
					session.created_at.dimmed(),
				);
			}
			Ok(())
		}
	}
}
