mod display;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use scriptmark_core::discovery::discover_submissions;
use scriptmark_core::grading::apply_curve;
use scriptmark_core::models::{CurveConfig, CurveMethod};
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

    /// Grade curve method
    #[arg(short, long, default_value = "sqrt", value_parser = parse_curve_method)]
    curve: CurveMethod,

    /// Curve range: lower,upper
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

    /// Grade curve method
    #[arg(short, long, default_value = "sqrt", value_parser = parse_curve_method)]
    curve: CurveMethod,

    /// Curve range: lower,upper
    #[arg(long, default_value = "60,100", value_parser = parse_range)]
    range: (f64, f64),
}

fn parse_curve_method(s: &str) -> Result<CurveMethod, String> {
    match s.to_lowercase().as_str() {
        "none" => Ok(CurveMethod::None),
        "linear" => Ok(CurveMethod::Linear),
        "sqrt" => Ok(CurveMethod::Sqrt),
        _ => Err(format!("unknown curve method: {s} (use none/linear/sqrt)")),
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

    // 5. Apply curve
    let curve_config = CurveConfig {
        method: args.curve,
        lower_bound: args.range.0,
        upper_bound: args.range.1,
    };
    let mut reports: Vec<_> = results.into_values().collect();
    apply_curve(&mut reports, &curve_config);
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

    let curve_config = CurveConfig {
        method: args.curve,
        lower_bound: args.range.0,
        upper_bound: args.range.1,
    };
    apply_curve(&mut reports, &curve_config);
    reports.sort_by(|a, b| a.student_id.cmp(&b.student_id));

    let report_refs: Vec<_> = reports.iter().collect();
    display::display_summary(&report_refs, &args.results.display().to_string());
    display::display_failures(&report_refs);
    display::display_stats(&report_refs);

    Ok(())
}
