use crate::views;
use crate::views::Component;
use common::defines::assignment::Assignment;
use common::defines::class::Class;
use common::defines::student::Student;
use cursive::view::{Nameable, Resizable};
use cursive::views::{Button, LinearLayout, Panel, StackView};
use cursive::{Cursive, CursiveRunnable, View};
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

fn build_class_list_view(classes: &[Class]) -> LinearLayout {
	let main_content_panel = Panel::new(StackView::new()
		.with_name(Component::ContentStack.as_ref()))
		.title("Content")
		.full_width()
		.full_height();


	let class_list_panel = views::get_class_list_panel(classes)
		.full_width()
		.full_height();


	let special_list_panel = views::get_class_special_list_panel()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });

	let button_panel = views::get_button_panel(
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


fn build_assignment_list_view(assignments: &[Assignment]) -> LinearLayout {
	let main_content_panel = Panel::new(StackView::new()
		.with_name(Component::ContentStack.as_ref()))
		.title("Content")
		.full_width()
		.full_height();


	let assignment_menu_view = views::get_assignment_menu_view(assignments)
		.full_width()
		.full_height();


	let special_list_panel = views::get_class_special_list_panel()
		.full_width()
		.full_height();


	let quit_button = Button::new("Quit", |s: &mut Cursive| { s.quit(); });
	let back_button = Button::new("Back", |s: &mut Cursive| { s.pop_layer(); });

	let button_panel = views::get_button_panel(
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
impl AppState {
	pub fn change_view(&mut self, view: ViewMode) {
		self.current_view_mode = view;
	}
	pub fn build_view_mode(&self) -> Box<dyn View> {
		match self.current_view_mode {
			ViewMode::ClassList => Box::new(build_class_list_view(&self.classes)),
			ViewMode::AssignmentList => {
				let selected = self.selected.clone();
				let selected_class = selected.class.unwrap_or_else(
					|| {
						panic!("No selected class");
					}
				);
				Box::new(build_assignment_list_view(&selected_class.assignments))
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
