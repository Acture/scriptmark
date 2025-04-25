pub struct FocusState {
	pub(crate) focus_chain: Vec<&'static str>,
	pub(crate) current_index: usize,
}