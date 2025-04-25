use clap::ValueEnum;
use flexi_logger::{Duplicate, FileSpec, Logger};
use log::error;
use serde::{Deserialize, Serialize};
use std::panic;
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

pub fn setup_panic_logging() {
	panic::set_hook(Box::new(|panic_info| {
		if let Some(location) = panic_info.location() {
			error!(
				"Panic occurred at {}:{}: {:?}",
				location.file(),
				location.line(),
				panic_info.payload().downcast_ref::<&str>()
			);
		} else {
			error!("Panic occurred: {:?}", panic_info.payload().downcast_ref::<&str>());
		}
	}));
}
pub fn init_logger(log_level: LogLevel, path: &Path, to_console: bool) {
	setup_panic_logging();
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