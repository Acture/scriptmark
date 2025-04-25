use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "grader", about = "Grading system")]
pub struct Args {
	#[arg(long, short, default_value = "config.toml")]
	pub config_path: PathBuf,
	#[arg(long, default_value_t = true)]
	pub override_config_if_failed: bool,

}