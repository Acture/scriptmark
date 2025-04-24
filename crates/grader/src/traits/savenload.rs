use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::error::Error;

pub trait SaveNLoad: Serialize + DeserializeOwned {
	fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>>;
	fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>>;
}


