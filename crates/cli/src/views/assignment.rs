use crate::state::{AppState, ViewMode};
use crate::views::{class, Component};
use crate::{utils, views};
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::traits::savenload::SaveNLoad;
use cursive::traits::{Nameable, Resizable, Scrollable};
use cursive::views::{Button, Dialog, EditView, LinearLayout, NamedView, Panel, ScrollView, SelectView, TextArea, TextView};
use cursive::Cursive;
use log::info;
use std::path::{Path, PathBuf};

type AssignmentViewMode = LinearLayout;
type AssignmentView = SelectView<Assignment>;
type AssignmentMenu = Panel<NamedView<ScrollView<AssignmentView>>>;

fn build_assignment_view(assignments: &[Assignment]) -> AssignmentView {
	let mut assignment_view = SelectView::new();
	for a in assignments.iter() {
		assignment_view.add_item(
			a.name.clone(),
			a.clone(),
		)
	}
	assignment_view
}

pub fn build_assignment_menu(assignments: &[Assignment]) -> AssignmentMenu {
	Panel::new(
		build_assignment_view(assignments)
			.on_submit(
				|s, a| {
					let state = s.user_data::<AppState>().expect("Failed to get app state");
					state.selected.assignment = Some(a.clone());
					state.change_view(ViewMode::AssignmentDetail);
					info!("View Mode Changed To: Assignment Detail");
					let new_view = state.build_view_mode();
					s.add_layer(
						new_view
					)
				}
			)
			.scrollable()
			.with_name(Component::AssignmentMenuView.as_ref())
	)
		.title("Assignment Menu")
}

pub fn build_assignment_detail_view_mode(selected_class: &Class, selected_assignment: &Assignment) -> LinearLayout {
	let assignment_detail = TextView::new(
		format!("{} - {}", selected_class.id, selected_assignment)
	);
	let student_menu = views::build_student_menu(&selected_class.students);
	let main_content_panel = Panel::new(
		LinearLayout::vertical()
			.child(assignment_detail)
			.child(student_menu.scrollable())
	)
		.title("Content")
		.full_width()
		.full_height();


	let assignment_menu_view = build_assignment_menu(&selected_class.assignments)
		.full_width()
		.full_height();


	let special_list_panel = build_assignment_detail_edit_menu()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });
	let back_button = Button::new("Back", |s: &mut Cursive| { s.pop_layer(); });

	let button_panel = views::build_button_menu(
		vec![back_button, quit_button]
	)
		.fixed_height(3);

	let left_column_layout = LinearLayout::vertical()
		.child(assignment_menu_view.full_height())
		.child(special_list_panel.full_height())
		.child(button_panel)
		.max_width(50)
		.full_height();

	LinearLayout::horizontal()
		.child(left_column_layout)
		.child(main_content_panel)
}

pub fn build_assignment_view_mode(assignments: &[Assignment]) -> AssignmentViewMode {
	let main_content_panel = Panel::new(LinearLayout::horizontal())
		.title("Content")
		.full_width()
		.full_height();


	let assignment_menu_view = build_assignment_menu(assignments)
		.full_width()
		.full_height();


	let special_list_panel = build_assignment_detail_edit_menu()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });
	let back_button = Button::new("Back", |s: &mut Cursive| { s.pop_layer(); });

	let button_panel = views::build_button_menu(
		vec![back_button, quit_button]
	)
		.fixed_height(3);

	let left_column_layout = LinearLayout::vertical()
		.child(assignment_menu_view.full_height())
		.child(special_list_panel.full_height())
		.child(button_panel)
		.max_width(50)
		.full_height();

	LinearLayout::horizontal()
		.child(left_column_layout)
		.child(main_content_panel)
}

pub fn build_assignment_detail_edit_menu() -> Panel<LinearLayout> {
	let mut s_v = LinearLayout::vertical();
	info!("build_assignment_edit_menu");

	let edit_assignment_button = Button::new("Edit Assignment", |s: &mut Cursive| {
		let selected_assignment = s.user_data::<AppState>().expect("Failed to load user data")
			.selected.assignment.clone().expect("Failed to get selected assignment");
		info!("Selected Assignment: {:?}", selected_assignment);
		let dialog = Dialog::new()
			.title("Edit Assignment")
			.content(
				LinearLayout::vertical()
					.child(TextView::new("Assignment Name:"))
					.child(EditView::new().content(
						selected_assignment.name.clone()
					).with_name("assignment_name"))
					.child(TextView::new("Points Possible:"))
					.child(TextArea::new().content(
						selected_assignment.points_possible.to_string()
					).with_name("points_possible"))
			)
			.button("确认", move |s| {
				let assignment_name = s.call_on_name("assignment_name", |view: &mut EditView| {
					view.get_content()
				}).expect("Failed to get assignment name");

				let points_possible = s.call_on_name("points_possible", |view: &mut TextArea| {
					view.get_content().parse::<f64>().expect("Failed to parse points possible")
				}).expect("Failed to get points possible");

				s.pop_layer();
				let state = s.user_data::<AppState>()
					.expect("Failed to get app state");
				let selected_class = state.selected.class.clone().expect("Failed to get selected class");
				let mut class_to_modify = state.classes.pop_if(
					|c| c.id == selected_class.id
				).expect("Failed to get class to modify");
				let mut assignment_to_modify = class_to_modify.assignments.pop_if(
					|a| a.name == selected_assignment.name.clone()
				).expect("Failed to get assignment to modify");
				assignment_to_modify.name = assignment_name.parse().expect("Failed to parse assignment name");
				assignment_to_modify.points_possible = points_possible;
				class_to_modify.assignments.push(
					assignment_to_modify
				);
				class_to_modify.save(&state.config.storage_dir).expect("Failed to save class to disk");

				state.classes.push(
					class_to_modify
				);
				let new_view = state.build_view_mode();

				s.pop_layer();
				s.add_layer(
					new_view
				);
			})
			.button("取消", |s| {
				s.pop_layer(); // 取消直接关闭
			});
		info!("Edit Assignment");
		s.add_layer(dialog);
	});
	s_v.add_child(
		edit_assignment_button
	);
	let add_submission_button = Button::new("Add Submission", |s: &mut Cursive| {
		let dialog = create_add_submission_dialog();
		s.add_layer(dialog);
	});


	s_v.add_child(
		add_submission_button
	);

	let group_submissions_button = Button::new("Group Submissions", |s: &mut Cursive| {
		let selected_class = s.user_data::<AppState>().expect("Failed to load user data")
			.selected.class.clone().expect("Failed to get selected class");
		let selected_assignment = s.user_data::<AppState>().expect("Failed to load user data")
			.selected.assignment.clone().expect("Failed to get selected assignment");
		let save_dir = s.user_data::<AppState>().expect("Failed to load user data")
			.config.data_dir.clone().join(selected_class.id).join(selected_assignment.name);
		info!("Save dir: {:?}", save_dir);
		utils::group_files_by_student(&save_dir, &selected_class.students);
		info!("Grouped files by student");
	});

	s_v.add_child(
		group_submissions_button
	);

	Panel::new(s_v)
		.title("Assignment Detail Edit Menu")
}

fn validate_submission_path(path: &Path) -> Result<(), String> {
	if !path.exists() {
		return Err("Submission path does not exist".to_string());
	}

	let extension = path.extension()
		.ok_or("Submission file has no extension")?
		.to_str()
		.ok_or("Invalid file extension encoding")?;

	if extension != "zip" {
		return Err("Submission path must be a zip file".to_string());
	}

	Ok(())
}

fn handle_add_submission_confirmation(s: &mut Cursive) {
	let submission_path = match s.call_on_name("submission_path", |view: &mut EditView| view.get_content()) {
		Some(content) => {
			match content.parse::<PathBuf>() {
				Ok(path) => {
					if let Err(msg) = validate_submission_path(&path) {
						class::show_error_message(s, &msg);
						return;
					}
					path
				}
				Err(err) => {
					class::show_error_message(s, &format!("Failed to parse submission path: {}", err));
					return;
				}
			}
		}
		None => {
			class::show_error_message(s, "Failed to get submission path field");
			return;
		}
	};

	s.pop_layer();

	let state = match s.user_data::<AppState>() {
		Some(state) => state,
		None => {
			class::show_error_message(s, "Failed to access application state");
			return;
		}
	};

	let data_dir = state.config.data_dir.clone();
	let selected_class = state.selected.class.clone().expect("Failed to get selected class");
	let selected_assignment = state.selected.assignment.clone().expect("Failed to get selected assignment");
	let save_dir = data_dir.join(selected_class.id).join(selected_assignment.name);
	info!("Save dir: {:?}", save_dir);
	utils::unzip_file(&submission_path, &save_dir).expect("Failed to unzip submission file");
	info!("Unzipped submission file");
	utils::group_files_by_student(&save_dir, &selected_class.students).expect("Failed to group files by student");
	info!("Grouped files by student");
}

fn create_add_submission_dialog() -> Dialog {
	Dialog::new()
		.title("Add New Submission")
		.content(
			LinearLayout::vertical()
				.child(TextView::new("Submission Path:"))
				.child(EditView::new().with_name("submission_path").fixed_width(30))
		)
		.button(
			"确认",
			|s| {
				handle_add_submission_confirmation(s);
			},
		)
		.button(
			"取消",
			|s| {
				s.pop_layer();
			},
		)
}
