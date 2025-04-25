use crate::state::{AppState, ViewMode};
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

type ClassSelectPanel = Panel<SelectView<Class>>;
type CursiveFn = dyn Fn(&mut Cursive);
type BoxedCursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type SpecialSelectPanel = Panel<SelectView<BoxedCursiveFn>>;
type ButtonSelectPanel = Panel<LinearLayout>;

fn get_assignment_view(assignments: &[Assignment]) -> ScrollView<SelectView<Assignment>> {
	let mut assignment_view = SelectView::new();
	for a in assignments.iter() {
		assignment_view.add_item(
			a.name.clone(),
			a.clone(),
		)
	}
	assignment_view.scrollable()
}

pub(crate) fn get_assignment_menu_view(assignments: &[Assignment]) -> NamedView<ScrollView<SelectView<Assignment>>> {
	let mut assignment_menu_view = get_assignment_view(assignments)
		.with_name(Component::AssignmentMenuView.as_ref());


	assignment_menu_view
}

fn get_assignment_content_view(assignments: &[Assignment]) -> ScrollView<SelectView<Assignment>> {
	let mut assignment_content_view = get_assignment_view(assignments);
	let assignments_owned = assignments.to_vec(); // 将切片克隆到一个新的 Vec 中

	assignment_content_view.get_inner_mut().set_on_submit(move |s, a| {
		s.call_on_name(Component::ContentStack.as_ref(), |content_stack: &mut NamedView<StackView>| {
			content_stack.with_view_mut(|content_stack| {
				while !content_stack.is_empty() {
					content_stack.pop_layer();
				}
			});
		});
		s.call_on_name(Component::TopStack.as_ref(), |top_stack: &mut NamedView<StackView>| {
			top_stack.with_view_mut(|top_stack| {
				let assignment_menu_view = get_assignment_menu_view(&assignments_owned);

				move_or_create_to_stack_front(
					top_stack,
					Component::AssignmentMenuView.as_ref(),
					assignment_menu_view,
				);
			});
			top_stack.take_focus(Direction::none()).expect("TODO: panic message");
		}).unwrap_or_else(|| {
			error!("Failed to set title")
		});
	});

	assignment_content_view
}

fn get_student_view(students: &[Student]) -> SelectView<Student> {
	let mut student_select_view = SelectView::new();
	for s in students.iter() {
		student_select_view.add_item(
			s.name.clone(),
			s.clone(),
		)
	}
	student_select_view
}

pub fn get_class_general_view_layout(class: &Class) -> NamedView<LinearLayout> {
	LinearLayout::vertical()
		.child(
			LinearLayout::horizontal()
				.child(Panel::new(get_assignment_content_view(&class.assignments).scrollable()).title(format!("Assignments ({})", class.assignments.len())))
				.child(Panel::new(get_student_view(&class.students).scrollable()).title(format!("Students ({})", class.students.len())))
		)
		.with_name(Component::ClassGeneralViewLayout.as_ref())
}

pub fn move_or_create_to_stack_front(stack: &mut StackView, name: &str, view: impl Resizable + ViewWrapper) {
	if let Some(layer_position) = stack.find_layer_from_name(name) {
		stack.move_to_front(layer_position);
	} else {
		stack.add_layer(
			view
		);
	}
}

pub fn get_class_list_panel(classes: &[Class]) -> ClassSelectPanel {
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
		.title("Class List")
}


pub fn get_class_special_list_panel() -> SpecialSelectPanel {
	Panel::new(SelectView::new()
		.h_align(HAlign::Center)
		.autojump()
		.item(
			"Add Class",
			Box::new(
				|s: &mut Cursive| {
					s.add_layer(
						Dialog::info("Add Class")
					);
				}
			) as BoxedCursiveFn,
		)
	)
		.title("Special")
}

pub fn get_button_panel(buttons: Vec<Button>) -> ButtonSelectPanel {
	let mut layout = LinearLayout::horizontal();
	for b in buttons {
		layout.add_child(b);
	}
	Panel::new(
		layout
	)
}