use common::defines;
use common::traits::savenload::SaveNLoad;
use std::path::Path;

pub fn load_saving(storage_path: &Path) -> Result<Vec<defines::class::Class>, Box<dyn std::error::Error>> {
	storage_path.read_dir()?.filter_map(
		|entry| {
			let path = entry.ok()?.path();
			match path.extension()?.to_str()?.eq_ignore_ascii_case("json") {
				true => {
					Some(defines::class::Class::load(&path))
				}
				false => None,
			}
		}
	).collect()
}