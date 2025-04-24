use crate::traits::savenload::SaveNLoad;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use toml::de::Error;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Debug, Serialize, Deserialize)]
pub struct Config {
	#[builder(default = PathBuf::from("data"))]
	data_dir: PathBuf,
	#[builder(default = PathBuf::from("data"))]
	storage_dir: PathBuf,
	#[builder(default = true)]
	move_roster: bool,
}

impl Config {
	pub fn load<P: AsRef<Path>>(config_path: P) -> Result<Config, Error> {
		let config_str = fs::read_to_string(config_path).expect("无法读取配置文件");
		toml::from_str(&config_str)
	}


	pub fn data_dir(&self) -> &Path {
		&self.data_dir
	}

	pub fn storage_dir(&self) -> &Path {
		&self.storage_dir
	}
}

pub fn prepare_config(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {

	// 检查配置文件是否存在
	match path.exists() {
		true => {
			Ok(Config::load(path)?)
		}
		false => {
			let default_config = Config::builder().build();
			let mut file = fs::File::create(path).expect("无法创建配置文件");
			let toml = toml::to_string(&default_config).expect("无法序列化配置文件");
			file.write_all(toml.as_bytes()).expect("无法写入配置文件");
			Ok(default_config)
		}
	}
}

impl SaveNLoad for Config {
	fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
		let mut file = fs::File::create(path)?;
		let toml_content = toml::to_string(&self)?;
		file.write_all(toml_content.as_bytes())?;
		Ok(())
	}

	fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
		let config_str = fs::read_to_string(path)?;
		Ok(toml::from_str(&config_str)?)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_config() {
		let config = Config::builder().build();
		assert_eq!(config.data_dir, Path::new("./data"));
	}
}
