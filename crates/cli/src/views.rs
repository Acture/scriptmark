use crate::state::{AppState, ViewMode};
use crate::views;
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use cursive::align::HAlign;
use cursive::direction::Direction;
use cursive::event::{Event, Key};
use cursive::traits::{Nameable, Resizable};
use cursive::view::{Scrollable, ViewWrapper};
use cursive::views::{Button, Dialog, LinearLayout, ListView, NamedView, Panel, ResizedView, ScrollView, SelectView, StackView, TextView};
use cursive::{Cursive, View};
use log::{error, info};
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
type ClassEditMenu = Panel<SelectView<BoxedCursiveFn>>;
type ButtonMenu = Panel<LinearLayout>;

type ClassViewMode = LinearLayout;
type AssignmentViewMode = LinearLayout;
type AssignmentView = ScrollView<SelectView<Assignment>>;
type AssignmentMenu = Panel<NamedView<AssignmentView>>;

fn build_assignment_view(assignments: &[Assignment]) -> AssignmentView {
	let mut assignment_view = SelectView::new();
	for a in assignments.iter() {
		assignment_view.add_item(
			a.name.clone(),
			a.clone(),
		)
	}
	assignment_view.scrollable()
}

pub fn build_assignment_menu(assignments: &[Assignment]) -> AssignmentMenu {
	Panel::new(
		build_assignment_view(assignments)
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
			let state = s.user_data::<AppState>().unwrap_or_else(
				|| panic!("Failed to get app state")
			);
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
pub fn build_assignment_view_mode(assignments: &[Assignment]) -> AssignmentViewMode {
	let main_content_panel = Panel::new(StackView::new()
		.with_name(Component::ContentStack.as_ref()))
		.title("Content")
		.full_width()
		.full_height();


	let assignment_menu_view = views::build_assignment_menu(assignments)
		.full_width()
		.full_height();


	let special_list_panel = views::build_class_edit_menu()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });
	let back_button = Button::new("Back", |s: &mut Cursive| { s.pop_layer(); });

	let button_panel = views::build_button_menu(
		vec![quit_button, back_button]
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


pub fn build_class_edit_menu() -> ClassEditMenu {
	Panel::new(SelectView::new()
		.h_align(HAlign::Center)
		.autojump()
		.item(
			"Add Class",
			Box::new(
				|s: &mut Cursive| {
					let state = s.user_data::<AppState>().unwrap_or_else(
						|| panic!("Failed to get app state")
					);
					// state.classes.push(
					// 	Class::
					// );
					s.add_layer(
						Dialog::info("Add Class")
					);
				}
			) as BoxedCursiveFn,
		)
	)
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