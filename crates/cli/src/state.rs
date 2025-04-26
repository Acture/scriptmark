use crate::config::Config;
use crate::views;
use crate::views::{assignment, class, Component};
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use cursive::view::{Nameable, Resizable};
use cursive::views::{Button, LinearLayout, Panel, StackView};
use cursive::{view, Cursive, CursiveRunnable, View};
use log::error;
use std::rc::Rc;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Default, Debug, Clone)]
pub struct Selected {
	#[builder(default)]
	pub class: Rc<Option<Class>>,
	#[builder(default)]
	pub student: Rc<Option<Student>>,
	#[builder(default)]
	pub assignment: Rc<Option<Assignment>>,
}

#[derive(TypedBuilder)]
pub struct AppState {
	pub config: Config,
	pub classes: Vec<Rc<Class>>,
	#[builder(default)]
	pub selected: Selected,
	#[builder(default=ViewMode::ClassList)]
	pub current_view_mode: ViewMode,
}


impl AppState {
	pub fn change_view(&mut self, view: ViewMode) {
		self.current_view_mode = view;
	}
	pub fn build_view_mode(&self) -> Box<dyn View> {
		match self.current_view_mode {
			ViewMode::ClassList => Box::new(class::build_class_view_mode(&self.classes)),
			ViewMode::AssignmentList => {
				let selected = self.selected.clone();
				let selected_class = selected.class.expect("No class selected");
				Box::new(assignment::build_assignment_view_mode(&selected_class.assignments))
			}
			ViewMode::AssignmentDetail => {
				let selected = self.selected.clone();
				let selected_class = selected.class.expect("No class selected");
				let selected_assignment = selected.assignment.expect("No assignment selected");
				Box::new(assignment::build_assignment_detail_view_mode(&selected_class, &selected_assignment))
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
