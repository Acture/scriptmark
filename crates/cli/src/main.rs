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
use log::{error, info, warn};
use std::path::Path;
use std::{env, iter};
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};
use views::Component;


fn main() {
	let args = Args::parse();

	let config = config::prepare_config(&args.config_path).unwrap_or_else(|e| {
		warn!("解析配置失败: {}", e);
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

	let main_content_view = StackView::new()
		.with_name(Component::MainContentView.as_ref());


	let main_content_panel = Panel::new(main_content_view)
		.title("Content")
		.full_width()
		.full_height()
		.with_name(Component::MainContentPanel.as_ref());

	let top_list_panel = views::get_top_list_panel(&classes)
		.title("Class List")
		.full_width()
		.full_height()
		.with_name(Component::TopListPanel.as_ref());

	let bottom_list_panel = views::get_bottom_list_panel()
		.title("Bottom")
		.full_width()
		.full_height()
		.with_name(Component::BottomListPanel.as_ref());

	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); })
		.with_name(Component::QuitButton.as_ref());

	let button_panel = Panel::new(quit_button)
		.fixed_height(3)
		.with_name(Component::ButtonPanel.as_ref());

	let left_column_layout = LinearLayout::vertical()
		.child(top_list_panel)
		.child(DummyView.fixed_height(1))
		.child(bottom_list_panel)
		.child(DummyView.fixed_height(1))
		.child(button_panel)
		.child(DummyView.fixed_height(1))
		.fixed_width(50)
		.full_height()
		.with_name(Component::LeftColumnLayout.as_ref());

	let main_layout = LinearLayout::horizontal()
		.child(left_column_layout)  // 左栏固定30列宽
		.child(main_content_panel)
		.with_name(Component::MainLayout.as_ref());

	let focus_state = focus::FocusState {
		focus_chain: vec![
			Component::TopListView.as_ref(),
			Component::BottomListView.as_ref(),
			Component::QuitButton.as_ref()
		],
		current_index: 0,
	};
	siv.set_user_data(focus_state);


	siv.add_global_callback(Key::Tab, |s: &mut Cursive| {
		let focus_name = {
			let state = s.user_data::<focus::FocusState>().unwrap();
			state.focus_chain[state.current_index].to_string()
		};

		s.add_layer(TextView::new(format!("Focus: {}", focus_name)));

		s.focus_name(&focus_name).ok();
	});


	siv.add_fullscreen_layer(main_layout);

	// Starts the event loop.
	siv.run();


	// init_logger();
	//
	// info!("开始加载班级信息...");
	// let classes = Class::prepare_class(&CONFIG.data_dir);
	// info!("班级信息加载完成");
	//
	// 'select_class: loop {
	// 	let selected_class = match tui::select_class(&classes) {
	// 		(tui::SelectStatus::Exit, _) => {
	// 			info!("退出程序");
	// 			break 'select_class;
	// 		}
	// 		(tui::SelectStatus::Return, _) => {
	// 			panic!("Unexpected return status");
	// 		}
	// 		(tui::SelectStatus::Selected, selected_class) => match selected_class {
	// 			Some(class) => class,
	// 			None => panic!("未找到对应班级"),
	// 		},
	// 	};
	// 	'select_assignment: loop {
	// 		match tui::select_assignment(selected_class) {
	// 			(tui::SelectStatus::Exit, _) => {
	// 				info!("退出程序");
	// 				exit(0);
	// 			}
	// 			(tui::SelectStatus::Return, _) => {
	// 				break 'select_assignment;
	// 			}
	// 			(tui::SelectStatus::Selected, selected_assignment_name) => {
	// 				let selected_assignment_name = match selected_assignment_name {
	// 					Some(name) => name,
	// 					None => panic!("未找到对应作业"),
	// 				};
	// 				info!("所选作业：{}", selected_assignment_name);
	// 				let (mut results, hash_map) = check::check_assignment(
	// 					selected_class,
	// 					&selected_assignment_name,
	// 					CONFIG.custom_solution,
	// 				);
	// 				'select_submissions_by_student: loop {
	// 					match tui::select_test_result(&mut results, &hash_map) {
	// 						(tui::SelectStatus::Exit, _) => {
	// 							info!("退出程序");
	// 							exit(0);
	// 						}
	// 						(tui::SelectStatus::Return, _) => {
	// 							break 'select_submissions_by_student;
	// 						}
	// 						(tui::SelectStatus::Selected, tr_res) => {
	// 							let selected_tr = match tr_res {
	// 								Some(selected_tr) => selected_tr,
	// 								None => panic!("未找到对应学生"),
	// 							};
	// 							'select_detail: loop {
	// 								match tui::select_detail(selected_tr) {
	// 									(tui::SelectStatus::Exit, _) => {
	// 										info!("退出程序");
	// 										exit(0);
	// 									}
	// 									(tui::SelectStatus::Return, _) => {
	// 										break 'select_detail;
	// 									}
	// 									(tui::SelectStatus::Selected, selected_detail) => {
	// 										match selected_detail {
	// 											Some(selected_detail) => {
	// 												println!("{:?}", selected_detail);
	// 											}
	// 											None => println!("未找到对应提交"),
	// 										}
	// 									}
	// 								}
	// 							}
	// 						}
	// 					}
	// 				}
	// 			}
	// 		}
	// 	}
	// }
}
