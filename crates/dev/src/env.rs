use once_cell::sync::Lazy;
use std::env;
use std::path::PathBuf;

pub fn get_dev_crate_dir() -> PathBuf {
	PathBuf::from(
		env!("CARGO_MANIFEST_DIR")
	)
}

pub static DEV_CRATE_DIR: Lazy<PathBuf> = Lazy::new(|| get_dev_crate_dir());

pub static CRATES_DIR: Lazy<PathBuf> = Lazy::new(|| {
	DEV_CRATE_DIR.parent().unwrap_or_else(|| panic!("Failed to get parent of dev crate dir {}", DEV_CRATE_DIR.display())).to_path_buf()
});

pub static PROJECT_DIR: Lazy<PathBuf> = Lazy::new(|| {
	CRATES_DIR.parent().unwrap_or_else(|| panic!("Failed to get parent of crates dir {}", CRATES_DIR.display())).to_path_buf()
});

pub static DATA_DIR: Lazy<PathBuf> = Lazy::new(|| {
	PROJECT_DIR.join("data")
});

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn test_get_dev_crate_dir() {
		assert!(DEV_CRATE_DIR.exists());
		println!("DEV_CRATE_DIR: {:?}", DEV_CRATE_DIR.display());
	}

	#[test]
	fn test_get_crates_dir() {
		assert!(CRATES_DIR.exists());
		println!("CRATES_DIR: {:?}", CRATES_DIR.display());
	}

	#[test]
	fn test_get_project_dir() {
		assert!(PROJECT_DIR.exists());
		println!("PROJECT_DIR: {:?}", PROJECT_DIR.display());
	}
	
	#[test]
	fn test_get_data_dir() {
		assert!(DATA_DIR.exists());
		println!("DATA_DIR: {:?}", DATA_DIR.display());
	}
}

