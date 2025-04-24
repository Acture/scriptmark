use crate::defines::class::Class;
use lazy_static::lazy_static;
use log::info;
use std::env;
use std::process::exit;
mod check;
mod config;
mod tui;

mod defines;
mod utils;

lazy_static! {
	static ref CONFIG: config::Config = config::prepare_config();
}

fn init_logger() {
	env_logger::Builder::new().parse_filters(&env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())) // 默认 info
		.init();
}

fn main() {
	init_logger();

	info!("开始加载班级信息...");
	let classes = Class::prepare_class(&CONFIG.data_dir);
	info!("班级信息加载完成");

	'select_class: loop {
		let selected_class = match tui::select_class(&classes) {
			(tui::SelectStatus::Exit, _) => {
				info!("退出程序");
				break 'select_class;
			}
			(tui::SelectStatus::Return, _) => {
				panic!("Unexpected return status");
			}
			(tui::SelectStatus::Selected, selected_class) => match selected_class {
				Some(class) => class,
				None => panic!("未找到对应班级"),
			},
		};
		'select_assignment: loop {
			match tui::select_assignment(selected_class) {
				(tui::SelectStatus::Exit, _) => {
					info!("退出程序");
					exit(0);
				}
				(tui::SelectStatus::Return, _) => {
					break 'select_assignment;
				}
				(tui::SelectStatus::Selected, selected_assignment_name) => {
					let selected_assignment_name = match selected_assignment_name {
						Some(name) => name,
						None => panic!("未找到对应作业"),
					};
					info!("所选作业：{}", selected_assignment_name);
					let (mut results, hash_map) = check::check_assignment(
						selected_class,
						&selected_assignment_name,
						CONFIG.custom_solution,
					);
					'select_submissions_by_student: loop {
						match tui::select_test_result(&mut results, &hash_map) {
							(tui::SelectStatus::Exit, _) => {
								info!("退出程序");
								exit(0);
							}
							(tui::SelectStatus::Return, _) => {
								break 'select_submissions_by_student;
							}
							(tui::SelectStatus::Selected, tr_res) => {
								let selected_tr = match tr_res {
									Some(selected_tr) => selected_tr,
									None => panic!("未找到对应学生"),
								};
								'select_detail: loop {
									match tui::select_detail(selected_tr) {
										(tui::SelectStatus::Exit, _) => {
											info!("退出程序");
											exit(0);
										}
										(tui::SelectStatus::Return, _) => {
											break 'select_detail;
										}
										(tui::SelectStatus::Selected, selected_detail) => {
											match selected_detail {
												Some(selected_detail) => {
													println!("{:?}", selected_detail);
												}
												None => println!("未找到对应提交"),
											}
										}
									}
								}
							}
						}
					}
				}
			}
		}
	}
}
