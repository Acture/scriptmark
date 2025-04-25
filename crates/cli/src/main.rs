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
use cursive::event::Key;
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


	let main_content_panel = Panel::new(StackView::new()
		.with_name(Component::ContentStack.as_ref()))
		.title("Content")
		.full_width()
		.full_height();

	let mut top_stack = StackView::new().with_name(Component::TopStack.as_ref());

	let class_list_panel = views::get_class_list_panel(&classes)
		.full_width()
		.full_height();

	top_stack.get_mut().add_layer(class_list_panel);

	let mut bottom_stack = StackView::new()
		.with_name(Component::BottomStack.as_ref());

	let special_list_panel = views::get_special_list_panel()
		.full_width()
		.full_height();

	bottom_stack.get_mut().add_layer(special_list_panel);


	let mut corner_stack = StackView::new().with_name(Component::CornerStack.as_ref());

	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });

	let button_panel = views::get_button_panel(
		vec![quit_button]
	)
		.fixed_height(3);

	corner_stack.get_mut().add_layer(button_panel);

	let left_column_layout = LinearLayout::vertical()
		.child(top_stack.full_height())
		.child(bottom_stack.full_height())
		.child(corner_stack)
		.max_width(50)
		.full_height();

	let main_layout = LinearLayout::horizontal()
		.child(left_column_layout)
		.child(main_content_panel);


	siv.add_fullscreen_layer(main_layout);

	// Starts the event loop.
	siv.run();
}
