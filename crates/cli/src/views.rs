use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use cursive::align::HAlign;
use cursive::traits::{Nameable, Resizable};
use cursive::view::{Scrollable, ViewWrapper};
use cursive::views::{Button, Dialog, LinearLayout, ListView, NamedView, Panel, ResizedView, SelectView, StackView, TextView};
use cursive::Cursive;
use log::error;
use std::alloc::Layout;
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};

#[derive(Debug, Clone, Copy, Display, EnumString, AsRefStr, IntoStaticStr)]
pub enum Component {
	ContentLayout,
	ContentPanel,
	TopListPanel,
	BottomListPanel,
	ButtonPanel,
	LeftColumnLayout,
	MainLayout,
}


fn get_assignment_view(assignments: &[Assignment]) -> SelectView<Assignment> {
	let mut assignment_select_view = SelectView::new();
	for a in assignments.iter() {
		assignment_select_view.add_item(
			a.name.clone(),
			a.clone(),
		)
	}
	assignment_select_view.set_on_submit(|s, a| {
		s.call_on_name(Component::TopListPanel.as_ref(), |outer: &mut NamedView<Panel<NamedView<LinearLayout>>>| {});
	});
	assignment_select_view
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

type TopListPanelViewType = NamedView<Panel<SelectView<Class>>>;
pub fn get_top_list_panel(classes: &[Class], title: &str) -> TopListPanelViewType {
	Panel::new(SelectView::new()
		.h_align(HAlign::Center)
		.autojump()
		.with_all(
			classes.iter().map(|c| {
				(format!("{} - {}", c.id, c.name), c.clone())
			})
		)
		.on_submit(|s, c| {
			s.call_on_name(Component::ContentPanel.as_ref(), |outer: &mut NamedView<Panel<NamedView<LinearLayout>>>| {
				let mut panel = outer.get_mut();
				let mut layout = panel.get_inner_mut().get_mut();
				layout.clear();
				layout.add_child(LinearLayout::vertical()
					.child(
						LinearLayout::horizontal()
							.child(Panel::new(get_assignment_view(&c.assignments).scrollable()).title(format!("Assignments ({})", c.assignments.len())))
							.child(Panel::new(get_student_view(&c.students).scrollable()).title(format!("Students ({})", c.students.len())))
					));
			}).unwrap_or_else(
				|| error!("Failed to set title")
			);
		})
	)
		.title(title)
		.with_name(Component::TopListPanel.as_ref())
}

type CursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type BoxedCursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type BottomListPanelViewType = NamedView<Panel<SelectView<BoxedCursiveFn>>>;
pub fn get_bottom_list_panel(title: &str) -> BottomListPanelViewType {
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
		.title(title)
		.with_name(Component::BottomListPanel.as_ref())
}

type ButtonPanelViewType = NamedView<Panel<LinearLayout>>;
pub fn get_button_panel(buttons: Vec<Button>) -> ButtonPanelViewType {
	let mut layout = LinearLayout::horizontal();
	for b in buttons {
		layout.add_child(b);
	}
	Panel::new(
		layout
	)
		.with_name(Component::ButtonPanel.as_ref())
}