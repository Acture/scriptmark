use crate::state::{AppState, Selected, ViewMode};
use crate::views;
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use common::traits::savenload::SaveNLoad;
use cursive::align::HAlign;
use cursive::direction::Direction;
use cursive::event::{Event, Key};
use cursive::traits::{Nameable, Resizable};
use cursive::view::{AnyView, IntoBoxedView, Scrollable, ViewWrapper};
use cursive::views::{Button, Dialog, EditView, LinearLayout, ListView, NamedView, Panel, ResizedView, ScrollView, SelectView, StackView, TextArea, TextView};
use cursive::{Cursive, View, With};
use log::{debug, error, info};
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};

#[derive(Debug, Clone, Copy, Display, EnumString, AsRefStr, IntoStaticStr)]
pub enum Component {
	TopStack,
	BottomStack,
	CornerStack,
	ContentStack,
	ClassGeneralViewLayout,
	AssignmentContentView,
	AssignmentMenuView,
}

type ClassMenu = Panel<SelectView<Class>>;
type CursiveFn = dyn Fn(&mut Cursive);
type BoxedCursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type ClassEditMenu = Panel<SelectView<EditView>>;
type ButtonMenu = Panel<LinearLayout>;

type ClassViewMode = LinearLayout;
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


	let class_list_panel = views::build_class_menu(classes)
		.full_width()
		.full_height();


	let special_list_panel = views::build_class_edit_menu()
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


pub fn build_student_menu(students: &[Student]) -> SelectView<Student> {
	let mut select: SelectView<Student> = SelectView::new();
	for student in students.iter() {
		select.add_item(
			student.name.clone(),
			student.clone(),
		)
	}
	select.set_on_submit(|s, student| {
		let state = s.user_data::<AppState>().expect("Failed to get app state");
		state.selected.student = Some(student.clone());
		state.change_view(ViewMode::StudentDetail);
		info!("View Mode Changed To: Student Detail");
		let new_view = state.build_view_mode();
		s.add_layer(
			new_view
		)
	});
	select
}


pub fn build_assignment_detail_view_mode(selected_class: &Class, selected_assignment: &Assignment) -> LinearLayout {
	let assignment_detail = TextView::new(
		format!("{} - {}", selected_class.id, selected_assignment)
	);
	let student_menu = build_student_menu(&selected_class.students);
	let main_content_panel = Panel::new(
		LinearLayout::vertical()
			.child(assignment_detail)
			.child(student_menu.scrollable())
	)
		.title("Content")
		.full_width()
		.full_height();


	let assignment_menu_view = views::build_assignment_menu(&selected_class.assignments)
		.full_width()
		.full_height();


	let special_list_panel = views::build_assignment_detail_edit_menu()
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


	let assignment_menu_view = views::build_assignment_menu(assignments)
		.full_width()
		.full_height();


	let special_list_panel = views::build_assignment_detail_edit_menu()
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
	Panel::new(s_v)
		.title("Assignment Detail Edit Menu")
}


pub fn build_class_edit_menu() -> Panel<LinearLayout> {
	let mut s_v = LinearLayout::vertical();


	let add_class_button = Button::new("Add Class", |s: &mut Cursive| {
		let dialog = Dialog::new()
			.title("Add New Class")
			.content(
				LinearLayout::vertical()
					.child(TextView::new("CSV Path:"))
					.child(EditView::new().with_name("csv_path").fixed_width(30))
			)
			.button("确认", |s| {
				let csv_path = s.call_on_name("csv_path", |view: &mut EditView| {
					view.get_content()
				}).expect("Failed to get csv path");

				s.pop_layer();
				let state = s.user_data::<AppState>()
					.expect("Failed to get app state");
				let new_class = Class::parse_from_csv(csv_path.parse().expect("Failed to get csv path"), state.config.storage_dir.clone(), None, None, true)
					.expect("Failed to parse class from csv");
				new_class.save(
					&state.config.storage_dir
				).expect("Failed to save class to disk");
				state.classes.push(
					new_class
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

		s.add_layer(dialog);
	});

	s_v.add_child(
		add_class_button
	);


	Panel::new(s_v)
		.title("Class Edit Menu")
}

pub fn build_button_menu(buttons: Vec<Button>) -> ButtonMenu {
	let mut layout = LinearLayout::horizontal();
	for b in buttons {
		layout.add_child(b);
	}
	Panel::new(
		layout
	)
}