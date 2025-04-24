use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, Serialize, Deserialize, TypedBuilder)]
pub struct Config {
	#[builder(default = PathBuf::from("./data"))]
	pub data_dir: PathBuf,
	#[builder(default = 42)]
	pub seed: u64,
	#[builder(default = true)]
	pub custom_solution: bool,
}

impl Config {
	pub fn load<P: AsRef<Path>>(config_path: P) -> Self {
		let config_str = fs::read_to_string(config_path).expect("无法读取配置文件");
		toml::from_str(&config_str).expect("无法解析配置文件")
	}
}

pub fn prepare_config() -> Result<Config, Box<dyn std::error::Error>
> {
	let config_path = "config.toml";

	// 检查配置文件是否存在
	match Path::new(config_path).exists() {
		true => {
			Ok(Config::load(config_path))
		}
		false => {
			let default_config = Config::builder().build();
			let mut file = fs::File::create(config_path).expect("无法创建配置文件");
			let toml = toml::to_string(&default_config).expect("无法序列化配置文件");
			file.write_all(toml.as_bytes()).expect("无法写入配置文件");
			Ok(default_config)
		}
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
