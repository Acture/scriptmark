#[macro_export]
macro_rules! rc_ref {
	($value:expr) => {
		Rc::new(RefCell::new($value))
	};
}