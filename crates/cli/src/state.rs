use crate::views;
use crate::views::Component;
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use cursive::view::{Nameable, Resizable};
use cursive::views::{Button, LinearLayout, Panel, StackView};
use cursive::{view, Cursive, CursiveRunnable, View};
use log::error;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Default, Debug, Clone)]
pub struct Selected {
	#[builder(default)]
	pub class: Option<Class>,
	#[builder(default)]
	pub student: Option<Student>,
	#[builder(default)]
	pub assignment: Option<Assignment>,
}

#[derive(TypedBuilder)]
pub struct AppState {
	pub classes: Vec<Class>,
	#[builder(default)]
	pub selected: Selected,
	#[builder(default=ViewMode::ClassList)]
	pub current_view_mode: ViewMode,
	#[builder(default)]
	pub current_view: Option<Box<dyn View>>,
}


impl AppState {
	pub fn change_view(&mut self, view: ViewMode) {
		self.current_view_mode = view;
	}
	pub fn build_view_mode(&self) -> Box<dyn View> {
		match self.current_view_mode {
			ViewMode::ClassList => Box::new(views::build_class_view_mode(&self.classes)),
			ViewMode::AssignmentList => {
				let selected = self.selected.clone();
				let selected_class = selected.class.unwrap_or_else(
					|| {
						panic!("No selected class");
					}
				);
				Box::new(views::build_assignment_view_mode(&selected_class.assignments))
			}
			_ => {
				error!("Not implemented view mode: {:?}", self.current_view_mode);
				unimplemented!()
			}
		}
	}
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
	ClassList,
	AssignmentList,
	AssignmentDetail,
	StudentDetail,
}
