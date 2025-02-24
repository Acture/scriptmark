use env_logger;
use log::info;
use std::env;

mod class;
mod utils;
mod config;
mod student;
mod assignment;

fn init_logger() {
	env_logger::Builder::new()
		.parse_filters(&env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())) // 默认 info
		.init();
}

fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let config = config::prepare_config();
	let classes = class::Class::prepare_class(config.data_dir);
	info!("班级信息加载完成: {:#?}", classes);

}
