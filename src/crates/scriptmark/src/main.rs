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
use scriptmark::record::{self, Evidence, Record, View};
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
	/// Preview student, file and function matching without running student programs
	Match(MatchArgs),
	/// Score a grading record again under the current policy, without running anything
	Rescore(RescoreArgs),
	/// Summarize a grading record as one of its revisions scored it
	Summarize(SummarizeArgs),
	/// Write a revision's grades as CSV
	Export(ExportArgs),
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
	/// Reuse the inputs and oracle answers frozen in FILE, verifying their sources
	#[arg(long, value_name = "FILE")]
	replay: Option<PathBuf>,

	/// Prepare fresh inputs and answers, replacing a different freeze beside --output
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

	/// Also write CSV tables to this directory: one row per case, and one per student's
	/// grade, beside the frozen inputs. The JSON is the record at --output.
	#[arg(short, long)]
	archive: Option<PathBuf>,

	/// Save results to SQLite database
	#[arg(long)]
	db: Option<PathBuf>,

	#[command(flatten)]
	frozen: FrozenArgs,

	/// Replace the grading record at --output even when it holds rescored revisions,
	/// discarding them. Long form only: there is no -f.
	#[arg(long)]
	force: bool,
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

	/// Replace the grading record at --output even when it holds rescored revisions,
	/// discarding them. Long form only: there is no -f.
	#[arg(long)]
	force: bool,
}

#[derive(Parser)]
struct MatchArgs {
	#[arg(required_unless_present = "canvas", conflicts_with = "canvas")]
	submissions: Vec<PathBuf>,
	#[arg(long)]
	canvas: Option<PathBuf>,
	#[arg(short = 't', long = "tests")]
	tests_dir: PathBuf,
	#[arg(short, long, default_value = "output/matches.json")]
	output: PathBuf,
	#[arg(short, long)]
	roster: Option<PathBuf>,
	#[arg(long)]
	assignment: Option<PathBuf>,
	#[arg(long, default_value = "python3")]
	python: String,

	/// Replace the grading record at --output even when it holds rescored revisions,
	/// discarding them. Long form only: there is no -f.
	#[arg(long)]
	force: bool,
}

/// Which score revision of a grading record to read.
#[derive(clap::Args)]
struct RevisionArg {
	/// The score revision to read; the latest by default
	#[arg(long, value_name = "N")]
	revision: Option<u32>,
}

#[derive(Parser)]
struct RescoreArgs {
	/// The grading record, as `grade` or `run` wrote it. The new revision is added to it.
	record: PathBuf,

	/// The assignment.toml holding the policy to score under. Defaults to the one the
	/// record was graded with, or one beside its tests directory.
	#[arg(long)]
	assignment: Option<PathBuf>,

	/// Save the new revision to SQLite database
	#[arg(long)]
	db: Option<PathBuf>,
}

#[derive(Parser)]
struct SummarizeArgs {
	/// Path to the grading record, as `grade`, `run` or `rescore` wrote it
	results: PathBuf,

	/// Path to roster CSV
	#[arg(short, long)]
	roster: Option<PathBuf>,

	#[command(flatten)]
	revision: RevisionArg,
}

#[derive(Parser)]
struct ExportArgs {
	/// Path to the grading record
	results: PathBuf,

	/// Where to write the grades: a `.csv` of the grades alone, or a `.xlsx` with the
	/// items, the cases behind the grades and the record they came from on sheets of
	/// their own
	#[arg(short, long, default_value = "grades.csv")]
	output: PathBuf,

	#[command(flatten)]
	revision: RevisionArg,
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

	/// Path to the grading record
	results: PathBuf,

	/// The score revision to push. Required when the record holds more than one.
	#[arg(long, value_name = "N")]
	revision: Option<u32>,
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
	/// Path to the grading record
	results: PathBuf,

	#[command(flatten)]
	revision: RevisionArg,

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
	/// Save a revision of a grading record as a session
	Save {
		/// The grading record
		record: PathBuf,
		#[command(flatten)]
		revision: RevisionArg,
		/// Database file path
		#[arg(long, default_value = "scriptmark.db")]
		db: PathBuf,
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
	purpose: Purpose,
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
			matching: Some(&declared.matching),
		},
	)
	.context("Failed to discover submissions")?;

	report_input(&input);

	// An Error diagnostic means the input cannot be trusted — a roster that disagrees with
	// itself about who a 学号 belongs to would attribute somebody's work to the wrong name.
	// Stop before running anything rather than producing results nobody should act on.
	let errors: Vec<String> = input.errors().map(|d| d.to_string()).collect();
	if purpose != Purpose::Preview && !errors.is_empty() {
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
		Commands::Match(args) => cmd_match(args),
		Commands::Rescore(args) => cmd_rescore(args),
		Commands::Summarize(args) => cmd_summarize(args),
		Commands::Export(args) => cmd_export(args),
		Commands::Canvas(cmd) => cmd_canvas(cmd).await,
		Commands::RosterPull(args) => cmd_roster_pull(args).await,
		Commands::GradesPush(args) => cmd_grades_push(args).await,
		Commands::Similarity(args) => cmd_similarity(args),
		Commands::Report(args) => cmd_report(args),
		Commands::Tui { db } => scriptmark::tui::run_tui(&db).context("TUI error"),
		Commands::Db(cmd) => cmd_db(cmd),
	}
}

/// A revision of the grading record at `path`: the latest unless one is named, and the
/// unscored evidence when nothing has scored it yet.
fn load_view(path: &Path, revision: Option<u32>) -> Result<(Record, View)> {
	let record = Record::load(path)?;
	let view = record.view(revision)?;
	Ok((record, view))
}

/// The revision a view shows, for consumers that need grades.
fn scored(view: &View, path: &Path) -> Result<u32> {
	view.revision.with_context(|| {
		format!(
			"{} has no score revision yet: score it with `scriptmark rescore {}`",
			path.display(),
			path.display()
		)
	})
}

/// How a summary names what it shows.
fn shown(path: &Path, view: &View) -> String {
	match view.revision {
		Some(n) => format!("{} (revision {n} of {})", path.display(), view.of),
		None => format!("{} (unscored)", path.display()),
	}
}

/// Prepare every test bundle, then run them against every student. A bundle that cannot
/// be prepared stops the run before any student is graded, and so do fresh inputs that
/// would replace other inputs frozen beside `output`.
///
/// Returns the reports, the inputs they were graded on, for `save_frozen` once the
/// results are written, and the interpreter that ran them.
async fn run_bundles(
	students: &[StudentSubmission],
	specs: Vec<TestSpec>,
	mut run_options: RunOptions,
	timeout: u64,
	output: &Path,
	options: &FrozenArgs,
) -> Result<(Vec<StudentReport>, Frozen, String)> {
	let generation = match &options.replay {
		Some(path) => Generation::Replay(Frozen::load(path)?),
		None => Generation::fresh(),
	};
	let executor = Arc::new(PythonExecutor::with_python_cmd(&run_options.python));
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
		// Even a batch without templates: what sits beside the results must be theirs.
		frozen::check_replaceable(&beside, &inputs, options.fresh)
			.map_err(anyhow::Error::msg)
			.context("refusing to grade")?;
		for (spec, templates) in &inputs.specs {
			for (case, made) in templates {
				if let (Some(seed), Some(SeedSource::Drawn)) = (made.seed, made.seed_source) {
					eprintln!(
						"  note: case '{case}' in '{spec}' drew seed {seed}. Write `seed = {seed}` in its [cases.parametrize.random] to keep these inputs, or, once this run is done, grade with --replay {}",
						beside.display()
					);
				}
			}
		}
	}
	run_options.python = executor.python_cmd().to_string();
	let python = run_options.python.clone();
	// Units run in their own process groups, so the terminal's Ctrl-C reaches only the
	// grader: take them down with it rather than leave them running to their timeouts.
	tokio::select! {
		reports = orchestrator::run_all(students, bundles.into(), executor, &run_options) => Ok((reports, inputs, python)),
		_ = tokio::signal::ctrl_c() => {
			scriptmark::runner::python::kill_all_units();
			anyhow::bail!("interrupted: every running unit was stopped")
		}
	}
}

/// Write the inputs a batch was graded on beside its results — after them, so an
/// interrupted or failed run replaces neither. A batch without templates has none, and
/// takes away any an earlier batch left there.
fn save_frozen(inputs: &Frozen, output: &Path) -> Result<()> {
	let path = frozen::beside(output);
	if inputs.is_empty() {
		if path.exists() {
			std::fs::remove_file(&path)
				.with_context(|| format!("failed to remove {}", path.display()))?;
		}
		return Ok(());
	}
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
	matching: scriptmark::matching::Config,
	/// Where it all was found, for the grading record.
	inputs: record::Inputs,
}

/// What a batch is prepared for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
	/// Grading: an input with errors is refused, and a Canvas bundle keeps its record of
	/// what was graded.
	Grade,
	/// A matching preview: an input with errors is shown, not refused.
	Preview,
	/// Checking evidence against the input as it is now: errors are refused, and nothing is
	/// written.
	Verify,
}

/// A path as an absolute one, without resolving links: what was found under it is recorded,
/// and must be found again under the same name whatever the working directory.
fn absolute(path: &Path) -> Result<PathBuf> {
	std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
}

/// Load the assignment and the specs, settle the items and the policy against each other,
/// then build the input — refusing a bad policy before a single student is run.
fn prepare_batch(
	submissions: &[PathBuf],
	canvas: Option<&PathBuf>,
	tests_dir: &Path,
	assignment_path: Option<&PathBuf>,
	roster: Option<&PathBuf>,
	purpose: Purpose,
) -> Result<Batch> {
	let tests_dir = absolute(tests_dir)?;
	let submissions = submissions
		.iter()
		.map(|p| absolute(p))
		.collect::<Result<Vec<_>>>()?;
	let canvas = canvas.map(|p| absolute(p)).transpose()?;
	let roster = roster.map(|p| absolute(p)).transpose()?;
	let assignment_path = assignment_path.map(|p| absolute(p)).transpose()?;

	let mut declared = assignment::load(assignment_path.as_deref(), &tests_dir)?;
	let specs = load_specs_from_dir(&tests_dir).context("Failed to load test specifications")?;
	println!("Loaded {} test specs", specs.len());

	let policy = assignment::settle(&mut declared.assignment, &declared.grading, &specs)?;
	declared.matching.validate(&specs)?;
	if policy.derived_items() {
		eprintln!(
			"  note: no [[items]] declared; each spec is an item worth 1 point. To weight \
			 them, add this to assignment.toml and edit the points:\n\n{}",
			assignment::items_toml(&declared.assignment.items)
		);
	}

	// Names, roster membership and submission state all come from the model, so there is
	// no separate roster merge afterwards.
	let (input, source) = match &canvas {
		Some(bundle) => (
			build_canvas_input(bundle, &declared, purpose)?,
			record::Source::Canvas {
				bundle: bundle.clone(),
			},
		),
		None => (
			build_local_input(&submissions, &declared, roster.as_ref(), purpose)?,
			record::Source::Local {
				dirs: submissions,
				roster,
			},
		),
	};
	Ok(Batch {
		input,
		specs,
		policy,
		matching: declared.matching,
		inputs: record::Inputs {
			tests: tests_dir,
			assignment: declared.path.map(|p| absolute(&p)).transpose()?,
			source,
		},
	})
}

/// Refuse to write a run's record over one holding rescored revisions — before anything
/// runs, so a refused run costs nothing — unless `--force` says to discard them. Even then
/// the record is replaced only once the run has succeeded.
fn check_output(output: &Path, force: bool) -> Result<()> {
	match record::check_replaceable(output) {
		Ok(()) => Ok(()),
		Err(why) if force => {
			eprintln!("  note: {why}; --force replaces it once this run succeeds");
			Ok(())
		}
		Err(why) => Err(anyhow::anyhow!(
			"{why}: move it aside, write this run elsewhere with --output, or pass --force to \
			 discard them"
		))
		.context("refusing to replace the grading record"),
	}
}

/// Run the batch and record what it found, unscored. The submissions are fingerprinted
/// before the run and checked after it, and the specs before they are prepared.
async fn execute(
	input: &AssignmentInput,
	specs: Vec<TestSpec>,
	inputs: record::Inputs,
	run_options: RunOptions,
	timeout: u64,
	output: &Path,
	frozen_args: &FrozenArgs,
) -> Result<(Record, Frozen)> {
	let spec_versions = record::spec_versions(&specs)?;
	let versions = record::submission_versions(&input.students)?;
	let matching = run_options.matching.clone();
	let (mut reports, frozen, python) = run_bundles(
		&input.students,
		specs,
		run_options,
		timeout,
		output,
		frozen_args,
	)
	.await?;
	record::seal(&mut reports, &input.students, versions)?;
	let bundle = record::bundle_version(
		spec_versions,
		timeout,
		python,
		&frozen,
		(!frozen.is_empty()).then(|| frozen::beside(output)),
	);
	let record = Record::new(Evidence {
		scriptmark: env!("CARGO_PKG_VERSION").to_string(),
		assignment: (&input.assignment).into(),
		inputs,
		attempt_policy: input.attempt_policy,
		matching,
		bundle,
		students: reports,
	})?;
	Ok((record, frozen))
}

/// Write a grading record where `output` says, its directory made first.
fn write_record(record: &Record, output: &Path) -> Result<()> {
	record
		.write(output)
		.with_context(|| format!("failed to write {}", output.display()))
}

async fn cmd_grade(args: GradeArgs) -> Result<()> {
	check_output(&args.output, args.force)?;
	let Batch {
		input,
		specs,
		policy,
		matching,
		inputs,
	} = prepare_batch(
		&args.submissions,
		args.canvas.as_ref(),
		&args.tests_dir,
		args.assignment.as_ref(),
		args.roster.as_ref(),
		Purpose::Grade,
	)?;

	let (mut record, frozen) = execute(
		&input,
		specs,
		inputs,
		RunOptions {
			python: args.python,
			matching: matching.clone(),
			concurrency: args
				.concurrency
				.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
		},
		args.timeout,
		&args.output,
		&args.frozen,
	)
	.await?;

	let items = &input.assignment.items;
	let revision = record.score(items, &policy)?;
	let view = record.view(Some(revision))?;
	let reports = &view.reports;

	// Display
	let report_refs: Vec<_> = reports.iter().collect();
	display::display_summary(&report_refs, &args.tests_dir.display().to_string());
	display::display_failures(&report_refs);
	display::display_stats(&report_refs);
	for warning in grading::diagnostics(reports, items) {
		eprintln!("  warning: {warning}");
	}

	// The grading record: the evidence and its first revision.
	write_record(&record, &args.output)?;
	println!("\nResults saved to {}", args.output.display());
	save_frozen(&frozen, &args.output)?;

	// Archive: the evidence per case, and the grades per student.
	if let Some(archive_dir) = &args.archive {
		std::fs::create_dir_all(archive_dir)?;
		let stem = args
			.tests_dir
			.file_name()
			.and_then(|n| n.to_str())
			.unwrap_or("results");
		let archive_path = archive_dir.join(format!("archive_{stem}.csv"));
		let grades_path = archive_dir.join(format!("grades_{stem}.csv"));
		if !frozen.is_empty() {
			let cases_path = archive_dir.join(format!("cases_{stem}.json"));
			frozen.write(&cases_path)?;
			println!("Inputs written to {}", cases_path.display());
		}
		let grades = scriptmark::export::grades(&record, revision)?;
		scriptmark::export::write_csv(&grades, std::fs::File::create(&grades_path)?)?;
		println!("Grades written to {}", grades_path.display());
		let cases = scriptmark::export::cases(reports);
		scriptmark::export::write_csv(&cases, std::fs::File::create(&archive_path)?)?;
		println!("Archived to {}", archive_path.display());
	}

	if let Some(db_path) = &args.db {
		save_to_db(db_path, &record, revision, input.roster.as_ref())?;
	}

	Ok(())
}

/// Save a revision as a database session, importing the roster it was graded with first.
/// Saving one revision twice finds the session it already has.
fn save_to_db(
	db_path: &Path,
	record: &Record,
	revision: u32,
	roster: Option<&scriptmark::roster::Roster>,
) -> Result<()> {
	let database = scriptmark::db::Database::open(db_path).context("Failed to open database")?;
	if let Some(roster) = roster {
		database
			.import_roster(roster)
			.context("Failed to import roster")?;
	}
	let saved = database
		.save_revision(record, revision)
		.context("Failed to save session to database")?;
	if saved.created {
		println!(
			"Saved to database: {} (session #{}, revision {revision})",
			db_path.display(),
			saved.id
		);
	} else {
		println!(
			"Revision {revision} is already in {} as session #{}",
			db_path.display(),
			saved.id
		);
	}
	Ok(())
}

async fn cmd_run(args: RunArgs) -> Result<()> {
	check_output(&args.output, args.force)?;
	// The policy is settled even though nothing is scored: a run whose results cannot be
	// graded should say so now, not after the class has run.
	let Batch {
		input,
		specs,
		matching,
		inputs,
		..
	} = prepare_batch(
		&args.submissions,
		args.canvas.as_ref(),
		&args.tests_dir,
		args.assignment.as_ref(),
		args.roster.as_ref(),
		Purpose::Grade,
	)?;

	// The same grading record `grade` writes, with no revision until it is scored.
	let (record, frozen) = execute(
		&input,
		specs,
		inputs,
		RunOptions {
			python: args.python,
			matching: matching.clone(),
			concurrency: args
				.concurrency
				.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
		},
		args.timeout,
		&args.output,
		&args.frozen,
	)
	.await?;

	write_record(&record, &args.output)?;
	println!(
		"Results saved to {}; score them with `scriptmark rescore {}`",
		args.output.display(),
		args.output.display()
	);
	save_frozen(&frozen, &args.output)?;

	Ok(())
}

fn cmd_match(args: MatchArgs) -> Result<()> {
	check_output(&args.output, args.force)?;
	let Batch {
		input,
		specs,
		matching,
		..
	} = prepare_batch(
		&args.submissions,
		args.canvas.as_ref(),
		&args.tests_dir,
		args.assignment.as_ref(),
		args.roster.as_ref(),
		Purpose::Preview,
	)?;
	let preview = scriptmark::matching::preview(input, &specs, &matching, &args.python)?;
	if let Some(parent) = args.output.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(&args.output, serde_json::to_string_pretty(&preview)?)?;
	println!(
		"Matching preview saved to {}: {} item(s) pending review, {} unattributed file(s)",
		args.output.display(),
		preview.pending.len(),
		preview.input.unmatched.len()
	);
	Ok(())
}

/// Score a grading record's evidence again under the policy as it is now, as a new
/// revision. Nothing runs: the record is refused unless its assignment, submissions, matching
/// and tests are still what they were, because otherwise its evidence describes something
/// that no longer exists.
fn cmd_rescore(args: RescoreArgs) -> Result<()> {
	let mut record = Record::load(&args.record)?;
	let inputs = record.evidence.inputs.clone();
	let (submissions, canvas, roster) = match &inputs.source {
		record::Source::Local { dirs, roster } => (dirs.clone(), None, roster.clone()),
		record::Source::Canvas { bundle } => (Vec::new(), Some(bundle.clone()), None),
	};
	let assignment = args.assignment.clone().or(inputs.assignment.clone());
	let Batch {
		input,
		specs,
		policy,
		matching,
		..
	} = prepare_batch(
		&submissions,
		canvas.as_ref(),
		&inputs.tests,
		assignment.as_ref(),
		roster.as_ref(),
		Purpose::Verify,
	)?;
	record.check(&record::Current {
		assignment: &input.assignment,
		attempt_policy: input.attempt_policy,
		matching: &matching,
		specs: &specs,
		students: &input.students,
	})?;

	let items = &input.assignment.items;
	let previous = record.latest().cloned();
	let revision = record.score(items, &policy)?;
	let view = record.view(Some(revision))?;

	let report_refs: Vec<_> = view.reports.iter().collect();
	display::display_summary(&report_refs, &shown(&args.record, &view));
	display::display_stats(&report_refs);
	for warning in grading::diagnostics(&view.reports, items) {
		eprintln!("  warning: {warning}");
	}
	let current = record.revision(revision)?;
	display::display_changes(
		previous.as_ref().map(|p| p.revision),
		revision,
		&record::diff(previous.as_ref(), current),
		view.reports.len(),
	);

	write_record(&record, &args.record)?;
	println!("\nRevision {revision} added to {}", args.record.display());

	if let Some(db_path) = &args.db {
		save_to_db(db_path, &record, revision, input.roster.as_ref())?;
	}
	Ok(())
}

fn cmd_summarize(args: SummarizeArgs) -> Result<()> {
	let (_, mut view) = load_view(&args.results, args.revision.revision)?;

	if let Some(roster_path) = &args.roster {
		let roster = load_roster(roster_path).context("Failed to load roster")?;
		for report in view.reports.iter_mut() {
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

	// Shown as the revision scored them: a summary never scores. `rescore` does, and
	// records what changed.
	let report_refs: Vec<_> = view.reports.iter().collect();
	display::display_summary(&report_refs, &shown(&args.results, &view));
	display::display_failures(&report_refs);
	display::display_stats(&report_refs);

	Ok(())
}

fn cmd_export(args: ExportArgs) -> Result<()> {
	let format = scriptmark::export::Format::of(&args.output)?;
	let (record, view) = load_view(&args.results, args.revision.revision)?;
	let revision = scored(&view, &args.results)?;
	let sheet = scriptmark::export::sheet(&record, revision, format)?;
	if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(&args.output, sheet)
		.with_context(|| format!("failed to write {}", args.output.display()))?;
	println!(
		"Grades of revision {revision} written to {}",
		args.output.display()
	);
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
fn build_canvas_input(
	bundle: &std::path::Path,
	declared: &Declared,
	purpose: Purpose,
) -> Result<AssignmentInput> {
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
	if purpose != Purpose::Preview && !errors.is_empty() {
		anyhow::bail!(
			"refusing to grade: {} problem(s) with the input\n  {}",
			errors.len(),
			errors.join("\n  ")
		);
	}

	// The record of which attempt was graded, and of every file's provenance, outlives the
	// process that produced it. Checking a record against the bundle writes nothing.
	if purpose == Purpose::Grade {
		bundle::save_input(bundle, &input)
			.with_context(|| format!("failed to write {}", bundle::input_path(bundle).display()))?;
	}

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
	// Everything about what to push is settled before Canvas is contacted.
	let record = Record::load(&args.results)?;
	if args.revision.is_none() && record.revisions.len() > 1 {
		anyhow::bail!(
			"{} holds {} score revisions; name the one to push with --revision",
			args.results.display(),
			record.revisions.len()
		);
	}
	// The record says which Canvas assignment its evidence belongs to; pushing it anywhere
	// else would publish one assignment's grades as another's.
	let graded_for = &record.evidence.assignment;
	for (flag, recorded, what) in [
		(args.course_id, graded_for.canvas_course_id, "course"),
		(
			args.assignment_id,
			graded_for.canvas_assignment_id,
			"assignment",
		),
	] {
		if let Some(recorded) = recorded
			&& recorded != flag
		{
			anyhow::bail!(
				"refusing to push: {} was graded for Canvas {what} {recorded}, not {flag}",
				args.results.display()
			);
		}
	}
	let view = record.view(args.revision)?;
	let revision = scored(&view, &args.results)?;

	// Only graded students are pushed — a real 0 included, a withheld grade never. The
	// Canvas user id is the one import recorded: parsing student_id as an integer would
	// either fail for every 学号 or, worse, post to whichever user held that number.
	let scriptmark::export::PushSet { grades, skipped } =
		scriptmark::export::grades_to_push(&view.reports)?;
	for (why, n) in &skipped {
		println!("Skipping {n} student(s): {why}");
	}

	let client = scriptmark::canvas::CanvasClient::new(&args.canvas_url)
		.context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;
	println!(
		"Pushing {} grades of revision {revision} (evidence {}) to Canvas assignment {}...",
		grades.len(),
		&record.digest[..12],
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
	let (_, view) = load_view(&args.results, args.revision.revision)?;
	let reports = view.reports;

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
		DbAction::Save {
			record,
			revision,
			db,
		} => {
			let (saved, view) = load_view(&record, revision.revision)?;
			let revision = scored(&view, &record)?;
			// The roster the record was graded with, when it had one, names its students.
			let roster = match &saved.evidence.inputs.source {
				record::Source::Local {
					roster: Some(path), ..
				} => Some(load_roster(path).context("Failed to load roster")?),
				_ => None,
			};
			save_to_db(&db, &saved, revision, roster.as_ref())
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
				"{:>4}  {:<20}  {:>3}  {:<12}  {:>8}  {:>8}  Date",
				"ID", "Assignment", "Rev", "Evidence", "Students", "Avg"
			);
			println!("{}", "-".repeat(90));
			for s in &sessions {
				println!(
					"{:>4}  {:<20}  {:>3}  {:<12}  {:>8}  {:>7}  {}",
					s.id.to_string().cyan(),
					s.assignment,
					s.revision,
					&s.evidence[..s.evidence.len().min(12)],
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
				"{:<15}  {:>3}  {:>24}  {:>10}  {:>8}/{:<8}  Date",
				"Assignment", "Rev", "Grade", "Pass Rate", "Passed", "Total"
			);
			println!("{}", "-".repeat(96));
			for (session, result) in &history {
				let grade_color = match result.fraction() {
					Some(f) if f >= 0.9 => "\x1b[32m",
					Some(f) if f >= 0.7 => "\x1b[34m",
					Some(_) => "\x1b[31m",
					None => "\x1b[2m",
				};
				let grade_text = result.grade_text();
				println!(
					"{:<15}  {:>3}  {}{:>24}\x1b[0m  {:>9.1}%  {:>8}/{}  {}",
					session.assignment,
					session.revision,
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
