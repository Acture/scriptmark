pub mod class;
pub mod assignment;

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

type CursiveFn = dyn Fn(&mut Cursive);
type BoxedCursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type ButtonMenu = Panel<LinearLayout>;


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


pub fn build_button_menu(buttons: Vec<Button>) -> ButtonMenu {
	let mut layout = LinearLayout::horizontal();
	for b in buttons {
		layout.add_child(b);
	}
	Panel::new(
		layout
	)
}