use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter};

pub trait Savable {
	fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>>;
}

pub trait Loadable: Sized {
	fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>>;
}

fn detect_format(path: &Path) -> Option<&str> {
	path.extension()?.to_str()
}

impl<T> Savable for T
where
	T: Serialize,
{
	fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
		let ext = detect_format(path.as_ref()).ok_or("Missing extension")?;
		let file = BufWriter::new(File::create(path.as_ref())?);
		match ext {
			"json" => serde_json::to_writer_pretty(file, self)?,
			"yaml" | "yml" => serde_yaml::to_writer(file, self)?,
			// CSV 支持另写封装
			_ => return Err(format!("Unsupported format: {ext}").into()),
		}
		Ok(())
	}
}

impl<T> Loadable for T
where
	T: DeserializeOwned,
{
	fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
		let ext = detect_format(path.as_ref()).ok_or("Missing extension")?;
		let file = BufReader::new(File::open(path.as_ref())?);
		let result = match ext {
			"json" => serde_json::from_reader(file)?,
			"yaml" | "yml" => serde_yaml::from_reader(file)?,
			_ => return Err(format!("Unsupported format: {ext}").into()),
		};
		Ok(result)
	}
}