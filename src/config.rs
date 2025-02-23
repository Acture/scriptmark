use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::{fs, path};
use toml;
use typed_builder::TypedBuilder;


#[derive(Debug, Serialize, Deserialize, TypedBuilder)]
pub struct Config {
	#[builder(default = PathBuf::from("./data"))]
	pub data_dir: PathBuf,
}

pub fn ensure_config_exists() {
	let config_path = "config.toml";

	// 检查配置文件是否存在
	if !Path::new(config_path).exists() {
		println!("配置文件不存在，正在创建默认 config.toml...");
		let default_config = Config::builder().build();
		let mut file = fs::File::create(config_path).expect("无法创建配置文件");
		let toml = toml::to_string(&default_config).expect("无法序列化配置文件");
		file.write_all(toml.as_bytes()).expect("无法写入配置文件");
	} else {
		println!("配置文件已存在: {}", config_path);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_config() {
		let config = Config::builder().build();
		assert_eq!(config.data_dir, path::Path::new("./data"));
	}
}
