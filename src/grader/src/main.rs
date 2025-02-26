use crate::defines::class::Class;
use env_logger;
use lazy_static::lazy_static;
use log::{info, log};
use std::env;
use lab;
mod check;
mod config;
mod tui;

mod defines;
mod utils;

lazy_static! {
	static ref CONFIG: config::Config = config::prepare_config();
}

fn init_logger() {
	env_logger::Builder::new()
		.parse_filters(&env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())) // 默认 info
		.init();
}

fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let classes = Class::prepare_class(&CONFIG.data_dir);
	info!("班级信息加载完成");

	tui::select_class(&classes);
}
