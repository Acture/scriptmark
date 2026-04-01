mod display;
mod report;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use scriptmark_core::discovery::discover_submissions;
use scriptmark_core::grading::apply_grading;
use scriptmark_core::models::{FormulaPolicy, GradingPolicy, TemplatePolicy};
use scriptmark_core::roster::load_roster;
use scriptmark_core::spec_loader::load_specs_from_dir;
use scriptmark_runner::orchestrator;
use scriptmark_runner::python::PythonExecutor;

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

#[derive(Parser)]
struct GradeArgs {
    /// Directories containing student submissions
    #[arg(required = true)]
    submissions: Vec<PathBuf>,

    /// Directory containing TOML test spec files
    #[arg(short = 't', long = "tests")]
    tests_dir: PathBuf,

    /// Output file for raw results (JSON)
    #[arg(short, long, default_value = "output/results.json")]
    output: PathBuf,

    /// Path to roster CSV (name,_,student_id)
    #[arg(short, long)]
    roster: Option<PathBuf>,

    /// Grading template: none, linear, sqrt, log, strict (default: sqrt)
    #[arg(short = 'g', long, default_value = "sqrt")]
    grading: String,

    /// Custom grading formula (Rhai expression). Overrides --grading.
    /// Variables: rate, passed, total, lint_score
    #[arg(long)]
    formula: Option<String>,

    /// Grade range: lower,upper
    #[arg(long, default_value = "60,100", value_parser = parse_range)]
    range: (f64, f64),

    /// Per-test timeout in seconds
    #[arg(long, default_value = "10")]
    timeout: u64,

    /// Max concurrent student executions
    #[arg(long)]
    concurrency: Option<usize>,

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
}

#[derive(Parser)]
struct RunArgs {
    /// Directories containing student submissions
    #[arg(required = true)]
    submissions: Vec<PathBuf>,

    /// Directory containing TOML test spec files
    #[arg(short = 't', long = "tests")]
    tests_dir: PathBuf,

    /// Output file for raw results (JSON)
    #[arg(short, long, default_value = "output/results.json")]
    output: PathBuf,

    /// Per-test timeout in seconds
    #[arg(long, default_value = "10")]
    timeout: u64,

    /// Max concurrent student executions
    #[arg(long)]
    concurrency: Option<usize>,

    /// Python interpreter command
    #[arg(long, default_value = "python3")]
    python: String,
}

#[derive(Parser)]
struct SummarizeArgs {
    /// Path to results JSON file
    results: PathBuf,

    /// Path to roster CSV
    #[arg(short, long)]
    roster: Option<PathBuf>,

    /// Grading template: none, linear, sqrt, log, strict (default: sqrt)
    #[arg(short = 'g', long, default_value = "sqrt")]
    grading: String,

    /// Custom grading formula (Rhai expression). Overrides --grading.
    /// Variables: rate, passed, total, lint_score
    #[arg(long)]
    formula: Option<String>,

    /// Grade range: lower,upper
    #[arg(long, default_value = "60,100", value_parser = parse_range)]
    range: (f64, f64),
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

fn build_grading_policy(grading: &str, formula: Option<&str>, range: (f64, f64)) -> GradingPolicy {
    if let Some(formula) = formula {
        GradingPolicy::Formula(FormulaPolicy {
            formula: formula.to_string(),
        })
    } else {
        GradingPolicy::Template(TemplatePolicy {
            template: grading.to_string(),
            lower: range.0,
            upper: range.1,
        })
    }
}

fn parse_range(s: &str) -> Result<(f64, f64), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err("range must be lower,upper (e.g. 60,100)".to_string());
    }
    let lower: f64 = parts[0].trim().parse().map_err(|e| format!("{e}"))?;
    let upper: f64 = parts[1].trim().parse().map_err(|e| format!("{e}"))?;
    Ok((lower, upper))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Grade(args) => cmd_grade(args).await,
        Commands::Run(args) => cmd_run(args).await,
        Commands::Summarize(args) => cmd_summarize(args),
        Commands::RosterPull(args) => cmd_roster_pull(args).await,
        Commands::GradesPush(args) => cmd_grades_push(args).await,
        Commands::Similarity(args) => cmd_similarity(args),
        Commands::Report(args) => cmd_report(args),
        Commands::Tui { db } => scriptmark_tui::run_tui(&db).context("TUI error"),
        Commands::Db(cmd) => cmd_db(cmd),
    }
}

async fn cmd_grade(args: GradeArgs) -> Result<()> {
    // 1. Discover submissions
    let submissions = discover_submissions(
        &args
            .submissions
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<_>>(),
        None,
    )
    .context("Failed to discover submissions")?;

    println!(
        "Found {} students in {} directories",
        submissions.student_count(),
        args.submissions.len()
    );

    // 2. Load test specs
    let specs =
        load_specs_from_dir(&args.tests_dir).context("Failed to load test specifications")?;
    println!("Loaded {} test specs", specs.len());

    // 3. Run tests
    let executor = PythonExecutor::with_python_cmd(&args.python);
    let mut results = orchestrator::run_all(
        &submissions,
        &specs,
        &executor,
        args.timeout,
        args.concurrency,
    )
    .await;

    // 4. Load roster and merge names
    if let Some(roster_path) = &args.roster {
        let roster = load_roster(roster_path).context("Failed to load roster")?;
        for (sid, report) in results.iter_mut() {
            if let Some(name) = roster.get(sid) {
                report.student_name = Some(name.clone());
            }
        }
    }

    // 5. Apply grading policy
    let policy = build_grading_policy(&args.grading, args.formula.as_deref(), args.range);
    let mut reports: Vec<_> = results.into_values().collect();
    apply_grading(&mut reports, &policy);
    reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));

    // 6. Display
    let report_refs: Vec<_> = reports.iter().collect();
    display::display_summary(&report_refs, &args.tests_dir.display().to_string());
    display::display_failures(&report_refs);
    display::display_stats(&report_refs);

    // 7. Save raw results
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&reports)?;
    std::fs::write(&args.output, &json)?;
    println!("\nResults saved to {}", args.output.display());

    // 8. Archive
    if let Some(archive_dir) = &args.archive {
        std::fs::create_dir_all(archive_dir)?;
        let stem = args
            .tests_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("results");
        let archive_path = archive_dir.join(format!("archive_{stem}.{}", args.format));

        match args.format.as_str() {
            "json" => {
                std::fs::write(&archive_path, &json)?;
            }
            "csv" => {
                let mut wtr = csv::Writer::from_path(&archive_path)?;
                wtr.write_record([
                    "student_name",
                    "student_id",
                    "spec_name",
                    "case_name",
                    "status",
                    "actual",
                    "expected",
                    "message",
                    "elapsed_ms",
                ])?;
                for report in &reports {
                    for test_result in &report.test_results {
                        for case in &test_result.cases {
                            wtr.write_record([
                                report.student_name.as_deref().unwrap_or(""),
                                &report.student_id,
                                &test_result.spec_name,
                                &case.case_name,
                                &format!("{:?}", case.status),
                                case.actual.as_deref().unwrap_or(""),
                                case.expected.as_deref().unwrap_or(""),
                                case.failure
                                    .as_ref()
                                    .map(|f| f.message.as_str())
                                    .unwrap_or(""),
                                &case.elapsed_ms.map(|ms| ms.to_string()).unwrap_or_default(),
                            ])?;
                        }
                    }
                }
                wtr.flush()?;
            }
            other => {
                eprintln!("Unknown archive format: {other}");
            }
        }
        println!("Archived to {}", archive_path.display());
    }

    // 9. Save to database if --db specified
    if let Some(db_path) = &args.db {
        let database = scriptmark_db::Database::open(db_path).context("Failed to open database")?;

        // Import roster if we loaded one
        if let Some(roster_path) = &args.roster
            && let Ok(roster) = scriptmark_core::roster::load_roster(roster_path)
        {
            let _ = database.import_roster(&roster);
        }

        let assignment = args
            .tests_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let session_id = database
            .save_session(assignment, &reports, None)
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
    let submissions = discover_submissions(
        &args
            .submissions
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<_>>(),
        None,
    )
    .context("Failed to discover submissions")?;

    println!(
        "Found {} students in {} directories",
        submissions.student_count(),
        args.submissions.len()
    );

    let specs =
        load_specs_from_dir(&args.tests_dir).context("Failed to load test specifications")?;
    println!("Loaded {} test specs", specs.len());

    let executor = PythonExecutor::with_python_cmd(&args.python);
    let results = orchestrator::run_all(
        &submissions,
        &specs,
        &executor,
        args.timeout,
        args.concurrency,
    )
    .await;

    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&results)?;
    std::fs::write(&args.output, &json)?;
    println!("Results saved to {}", args.output.display());

    Ok(())
}

fn cmd_summarize(args: SummarizeArgs) -> Result<()> {
    let content = std::fs::read_to_string(&args.results).context("Failed to read results file")?;
    let mut reports: Vec<scriptmark_core::models::StudentReport> =
        serde_json::from_str(&content).context("Failed to parse results JSON")?;

    if let Some(roster_path) = &args.roster {
        let roster = load_roster(roster_path).context("Failed to load roster")?;
        for report in reports.iter_mut() {
            if let Some(name) = roster.get(&report.student_id) {
                report.student_name = Some(name.clone());
            }
        }
    }

    let policy = build_grading_policy(&args.grading, args.formula.as_deref(), args.range);
    apply_grading(&mut reports, &policy);
    reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));

    let report_refs: Vec<_> = reports.iter().collect();
    display::display_summary(&report_refs, &args.results.display().to_string());
    display::display_failures(&report_refs);
    display::display_stats(&report_refs);

    Ok(())
}

async fn cmd_roster_pull(args: RosterPullArgs) -> Result<()> {
    let client = scriptmark_canvas::CanvasClient::new(&args.canvas_url)
        .context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;

    println!("Pulling roster from Canvas course {}...", args.course_id);
    let roster = client
        .pull_roster(args.course_id)
        .await
        .context("Failed to pull roster from Canvas")?;

    println!("Found {} students", roster.len());

    scriptmark_canvas::CanvasClient::save_roster_csv(&roster, &args.output)
        .context("Failed to save roster CSV")?;

    println!("Roster saved to {}", args.output.display());
    Ok(())
}

async fn cmd_grades_push(args: GradesPushArgs) -> Result<()> {
    let client = scriptmark_canvas::CanvasClient::new(&args.canvas_url)
        .context("Failed to create Canvas client (is CANVAS_TOKEN set?)")?;

    let content = std::fs::read_to_string(&args.results).context("Failed to read results file")?;
    let reports: Vec<scriptmark_core::models::StudentReport> =
        serde_json::from_str(&content).context("Failed to parse results JSON")?;

    // Build grades map: try to parse student_id as u64 (Canvas user ID)
    let mut grades = std::collections::HashMap::new();
    for report in &reports {
        if let Some(grade) = report.final_grade {
            if let Ok(uid) = report.student_id.parse::<u64>() {
                grades.insert(uid, grade);
            } else {
                eprintln!(
                    "Warning: cannot push grade for '{}' — student_id is not a Canvas user ID",
                    report.student_id
                );
            }
        }
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
    use scriptmark_core::similarity::compare_submissions;

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
    let reports: Vec<scriptmark_core::models::StudentReport> =
        serde_json::from_str(&content).context("Failed to parse results JSON")?;

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
        Some(scriptmark_core::similarity::compare_submissions(
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
                scriptmark_db::Database::open(&path).context("Failed to initialize database")?;
            println!("Database initialized: {}", path.display());
            Ok(())
        }
        DbAction::ImportRoster { roster, db } => {
            let database = scriptmark_db::Database::open(&db).context("Failed to open database")?;
            let roster_map = scriptmark_core::roster::load_roster(&roster)
                .context("Failed to load roster CSV")?;
            let count = database
                .import_roster(&roster_map)
                .context("Failed to import roster")?;
            println!("Imported {} students into {}", count, db.display());
            Ok(())
        }
        DbAction::Sessions { db } => {
            let database = scriptmark_db::Database::open(&db).context("Failed to open database")?;
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
                    "{:>4}  {:<20}  {:>8}  {:>7.1}  {}",
                    s.id.to_string().cyan(),
                    s.assignment,
                    s.student_count,
                    s.avg_grade,
                    s.created_at.dimmed(),
                );
            }
            Ok(())
        }
        DbAction::History { student_id, db } => {
            let database = scriptmark_db::Database::open(&db).context("Failed to open database")?;
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
                "{:<15}  {:>8}  {:>10}  {:>8}/{:<8}  Date",
                "Assignment", "Grade", "Pass Rate", "Passed", "Total"
            );
            println!("{}", "-".repeat(75));
            for (session, result) in &history {
                let grade_color = if result.final_grade >= 90.0 {
                    "\x1b[32m"
                } else if result.final_grade >= 70.0 {
                    "\x1b[34m"
                } else {
                    "\x1b[31m"
                };
                println!(
                    "{:<15}  {}{:>7.1}\x1b[0m  {:>9.1}%  {:>8}/{}  {}",
                    session.assignment,
                    grade_color,
                    result.final_grade,
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
