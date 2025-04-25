use clap::ValueEnum;
use flexi_logger::{Duplicate, FileSpec, Logger};
use serde::{Deserialize, Serialize};
use std::path::Path;
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};

#[derive(
	Debug,
	Clone,
	Copy,
	Display,
	EnumString,
	AsRefStr,
	IntoStaticStr,
	ValueEnum,
	Serialize,
	Deserialize
)]
pub enum LogLevel {
	Debug,
	Info,
	Warn,
	Error,
}

pub fn init_logger(log_level: LogLevel, path: &Path, to_console: bool) {
	match to_console {
		true => Logger::try_with_env_or_str(log_level.as_ref()).unwrap()
			.log_to_file(FileSpec::default().directory(path).suppress_timestamp())
			.duplicate_to_stdout(Duplicate::All)
			.start().unwrap(),
		false => Logger::try_with_env_or_str(log_level.as_ref()).unwrap()
			.log_to_file(FileSpec::default().suppress_timestamp().directory(path))
			.start().unwrap(),
	};
}