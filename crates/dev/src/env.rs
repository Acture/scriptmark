use dotenvy::dotenv;
use once_cell::sync::Lazy;
use std::env;

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
	dotenv().ok(); // 自动加载 .env
	Config {
		db_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
		log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
	}
});


