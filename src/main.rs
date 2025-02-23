use crate::config::ensure_config_exists;

mod class;
mod utils;
mod config;
mod student;

fn main() {
	ensure_config_exists();
	println!("程序启动...");
}
