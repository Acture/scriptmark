use common::defines::class::Class;
use cursive::align::HAlign;
use cursive::traits::Nameable;
use cursive::views::{Dialog, NamedView, Panel, SelectView, StackView, TextView};
use cursive::Cursive;
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};

#[derive(Debug, Clone, Copy, Display, EnumString, AsRefStr, IntoStaticStr)]
pub enum Component {
	MainContentView,
	MainContentPanel,
	TopListView,
	TopListPanel,
	BottomListView,
	BottomListPanel,
	QuitButton,
	ButtonPanel,
	LeftColumnLayout,
	MainLayout,
}

pub fn get_top_list_panel(classes: &[Class]) -> Panel<NamedView<SelectView<Class>>> {
	Panel::new(SelectView::new()
		.h_align(HAlign::Center)
		.autojump()
		.with_all(
			classes.iter().map(|c| {
				(format!("{} - {}", c.id, c.name), c.clone())
			})
		)
		.on_select(
			|s, c| {
				let content = format!("Selected: {}", c.name);
				s.call_on_name(Component::MainContentView.as_ref(), |mc: &mut TextView| {
					mc.set_content(content);
				}).unwrap();
			}
		)
		.on_submit(|s, c| {
			let main_content = s.call_on_name(Component::MainContentView.as_ref(), |mc: &mut StackView| {
				mc.add_layer(TextView::new(format!("Submitted: {}", c.name)).h_align(HAlign::Center))
			});
		})
		.with_name(Component::TopListView.as_ref()))
}

type CursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;
type BoxedCursiveFn = Box<dyn Fn(&mut Cursive) + Send + Sync>;

pub fn get_bottom_list_panel() -> Panel<NamedView<SelectView<BoxedCursiveFn>>> {
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
		.with_name(Component::BottomListView.as_ref())
	)
}