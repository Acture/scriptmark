use crate::state::{AppState, ViewMode};
use crate::views;
use crate::views::Component;
use common::defines::class::Class;
use common::traits::savenload::SaveNLoad;
use cursive::align::HAlign;
use cursive::traits::{Nameable, Resizable};
use cursive::views::{Button, Dialog, EditView, LinearLayout, Panel, SelectView, StackView, TextView};
use cursive::Cursive;
use log::info;
use std::path::{Path, PathBuf};

type ClassMenu = Panel<SelectView<Class>>;
type ClassEditMenu = Panel<SelectView<EditView>>;
type ClassViewMode = LinearLayout;

pub fn build_class_menu(classes: &[Class]) -> ClassMenu {
	Panel::new(SelectView::new()
		.h_align(HAlign::Center)
		.autojump()
		.with_all(
			classes.iter().map(|c| {
				(format!("{} - {}", c.id, c.name), c.clone())
			})
		)
		.on_submit(|s, c| {
			let state = s.user_data::<AppState>().expect("Failed to get app state");
			state.selected.class = Some(c.clone());
			state.change_view(ViewMode::AssignmentList);
			info!("View Mode Changed To: Assignment List");
			let new_view = state.build_view_mode();
			info!("Assignment List");
			s.add_layer(
				new_view
			);
		})
	)
		.title("Class Menu")
}

pub fn build_class_view_mode(classes: &[Class]) -> ClassViewMode {
	let main_content_panel = Panel::new(StackView::new()
		.with_name(Component::ContentStack.as_ref()))
		.title("Content")
		.full_width()
		.full_height();


	let class_list_panel = build_class_menu(classes)
		.full_width()
		.full_height();


	let special_list_panel = build_class_edit_menu()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });

	let button_panel = views::build_button_menu(
		vec![quit_button]
	)
		.fixed_height(3);

	let left_column_layout = LinearLayout::vertical()
		.child(class_list_panel.full_height())
		.child(special_list_panel.full_height())
		.child(button_panel)
		.max_width(50)
		.full_height();

	LinearLayout::horizontal()
		.child(left_column_layout)
		.child(main_content_panel)
}

fn create_add_class_dialog() -> Dialog {
	Dialog::new()
		.title("Add New Class")
		.content(
			LinearLayout::vertical()
				.child(TextView::new("CSV Path:"))
				.child(EditView::new().with_name("csv_path").fixed_width(30))
		)
		.button("确认", handle_add_class_confirmation)
		.button("取消", |s| { s.pop_layer(); })
}
fn handle_add_class_confirmation(s: &mut Cursive) {
	// 获取CSV路径
	let csv_path = match s.call_on_name("csv_path", |view: &mut EditView| view.get_content()) {
		Some(content) => content,
		None => {
			show_error_message(s, "Failed to get CSV path field");
			return;
		}
	};

	// 关闭对话框
	s.pop_layer();

	// 获取应用状态
	let state = match s.user_data::<AppState>() {
		Some(state) => state,
		None => {
			show_error_message(s, "Failed to access application state");
			return;
		}
	};

	// 解析CSV并创建新班级
	let parse_result = Class::parse_from_csv(
		PathBuf::from(&*csv_path),
		state.config.storage_dir.clone(),
		None,
		None,
		true,
	);

	// 处理解析结果
	match parse_result {
		Ok(new_class) => {
			// 保存班级并更新视图
			if let Err(err) = new_class.save(&state.config.storage_dir) {
				show_error_message(s, &format!("Failed to save class: {}", err));
				return;
			}

			state.classes.push(new_class);

			// 更新视图
			let new_view = state.build_view_mode();
			s.pop_layer();
			s.add_layer(new_view);
		}
		Err(err) => {
			show_error_message(s, &format!("Failed to parse class from CSV: {}", err));
		}
	}
}
pub fn show_error_message(s: &mut Cursive, message: &str) {
	s.add_layer(
		Dialog::around(TextView::new(message))
			.title("Error")
			.button("OK", |s| { s.pop_layer(); })
	);
}


pub fn build_class_edit_menu() -> Panel<LinearLayout> {
	let mut s_v = LinearLayout::vertical();


	let add_class_button = Button::new("Add Class", |s: &mut Cursive| {
		// 创建添加班级对话框
		let dialog = create_add_class_dialog();
		s.add_layer(dialog);
	});


	s_v.add_child(
		add_class_button
	);


	Panel::new(s_v)
		.title("Class Edit Menu")
}