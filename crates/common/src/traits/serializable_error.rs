use serde::{Deserialize, Serialize};

pub trait SerializableError: std::error::Error + Serialize + for<'de> Deserialize<'de> + Send + Sync {}

impl<T> SerializableError for T
where
	T: std::error::Error + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
{}