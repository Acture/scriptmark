use csv::{ReaderBuilder, StringRecord};
use log::trace;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder, Serialize, Deserialize, Clone)]
pub struct Student {
	pub name: String,
	pub id: String,
	pub sis_login_id: String,
}

impl PartialEq for Student {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}

impl Eq for Student {}

impl Hash for Student {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state);
	}
}

impl Student {
	pub fn load_from_roster<P: AsRef<Path>>(path: P) -> Vec<Student> {
		let path = path.as_ref();
		let file = fs::File::open(path).expect("无法打开文件");
		let mut rdr = ReaderBuilder::new()
			.has_headers(true) // 表示文件具有标题行
			.from_reader(file);
		let headers = rdr.headers().expect("读取表头失败").clone();
		let required_headers = vec!["Student", "ID", "SIS Login ID"];

		let header_map: HashMap<&str, usize> =
			headers.iter().enumerate().map(|(i, h)| (h, i)).collect();

		let indices: Vec<usize> = required_headers
			.into_iter()
			.map(|h| header_map.get(h).copied().expect(&format!("缺少列: {}", h)))
			.collect();

		let [name_index, id_index, sis_login_index]: [usize; 3] =
			indices.try_into().expect("索引数量不匹配");

		let mut record_iter = rdr.records();
		trace!("跳过第二行");
		record_iter.next();

		record_iter
			.filter_map(|result| {
				let record = result.expect("读取 CSV 行失败");
				match Student::from_record(&record, name_index, id_index, sis_login_index) {
					Some(student) if student.name != "测验学生" => Some(student), // 只保留 name ≠ "John Doe"
					Some(_) => {
						log::trace!("过滤掉学生: {:?}", record);
						None
					}
					None => {
						log::warn!("无法解析学生记录: {:?}", record);
						None
					}
				}
			})
			.collect()
	}

	fn from_record(
		record: &StringRecord,
		name_index: usize,
		id_index: usize,
		sis_login_index: usize,
	) -> Option<Student> {
		if record.len() < 3 {
			return None; // 如果列数不足，则跳过这条记录
		}

		Some(Student {
			name: record.get(name_index)?.to_string(),
			id: record.get(id_index)?.to_string(),
			sis_login_id: record.get(sis_login_index)?.to_string(),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::class::Class;
	use crate::config::Config;
	use std::path::PathBuf;

	#[test]
	fn test_load_student() {
		let config = Config::builder().build();
		assert_eq!(config.data_dir, PathBuf::from("./data"));
		let classes = Class::load_class("./data");
		classes.iter().for_each(|class| {
			let students = Student::load_from_roster(class.roster_path());
			let test_student = students.iter().find(|s| s.name == "测验学生");
			assert!(test_student.is_none());
		});
	}
}
