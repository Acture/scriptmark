use std::path::Path;

pub trait Savable {
	fn save(&self, save_dir: &Path) -> Result<(), Box<dyn std::error::Error>>;
}