use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::error::Error;

pub trait SaveNLoad: Serialize + DeserializeOwned {
	fn save(&self, path: &Path) -> Result<(), Box<dyn Error>>;
	fn load(path: &Path) -> Result<Self, Box<dyn Error>>;
}


