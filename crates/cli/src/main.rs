mod tui;
mod args;
mod views;
mod focus;
mod utils;
mod logger;
mod config;

use crate::args::Args;
use clap::Parser;
use common::traits::savenload::SaveNLoad;
use cursive::align::{HAlign, VAlign};
use cursive::view::{Nameable, Resizable};
use cursive::views::{Button, Dialog, DummyView, LinearLayout, NamedView, Panel, ResizedView, SelectView, StackView, TextView};
use cursive::{Cursive, With};
use log::{debug, error, info, warn};
use std::path::Path;
use std::{env, iter};
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};
use views::Component;


fn main() {
	let args = Args::parse();

	let config = config::prepare_config(&args.config_path).unwrap_or_else(|e| {
		println!("解析配置失败: {}", e);
		let c = config::Config::builder()
			.build();
		c.save(&args.config_path).expect("Failed to save config");
		c
	});
	logger::init_logger(
		config.log_level,
		&config.log_dir,
		config.log_to_console,
	);

	info!("数据目录：{}, 存储目录：{}", config.data_dir().to_string_lossy(), config.storage_dir().to_string_lossy());

	let classes = utils::load_saving(config.storage_dir()).unwrap_or_else(|e| {
		error!("{}", e);
		vec![]
	});


	info!("{} classes loaded", classes.len());

	let mut siv = cursive::default();


	let content_inner_layout = LinearLayout::vertical().with_name(
		Component::ContentLayout.as_ref()
	);


	let content_outer_panel = Panel::new(content_inner_layout)
		.title("Content")
		.with_name(Component::ContentPanel.as_ref())
		.full_width()
		.full_height();


	let top_list_panel = views::get_top_list_panel(&classes, "Class List")
		.full_width()
		.full_height();

	let bottom_list_panel = views::get_bottom_list_panel("Special Action")
		.full_width()
		.full_height();

	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });

	let button_panel = views::get_button_panel(
		vec![quit_button]
	)
		.fixed_height(3);

	let left_column_layout = LinearLayout::vertical()
		.child(top_list_panel)
		.child(bottom_list_panel)
		.child(button_panel)
		.with_name(Component::LeftColumnLayout.as_ref())
		.max_width(50)
		.full_height();

	let main_layout = LinearLayout::horizontal()
		.child(left_column_layout)
		.child(content_outer_panel)
		.with_name(Component::MainLayout.as_ref());


	siv.add_fullscreen_layer(main_layout);

	// Starts the event loop.
	siv.run();


}
