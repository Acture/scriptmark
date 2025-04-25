use common::defines;
use common::defines::student::Student;
use common::traits::savenload::SaveNLoad;
use log::{info, warn};
use std::fs;
use std::fs::File;
use std::path::Path;
use zip::ZipArchive;

pub fn load_saving(storage_path: &Path) -> Result<Vec<defines::class::Class>, Box<dyn std::error::Error>> {
	Ok(storage_path.read_dir()?.filter_map(
		|entry| {
			let path = entry.ok()?.path();
			match path.extension()?.to_str()?.eq_ignore_ascii_case("json") {
				true => match defines::class::Class::load(&path) {
					Ok(class) => Some(class),
					Err(e) => {
						warn!("Error loading class: {}", e);
						None
					}
				}
				false => None,
			}
		}
	).collect())
}

pub fn unzip_file(zip_path: &Path, output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
	let file = File::open(zip_path)?;
	let mut archive = ZipArchive::new(file)?;

	for i in 0..archive.len() {
		let mut file = archive.by_index(i)?;
		let outpath = output_dir.join(file.name());

		if (*file.name()).ends_with('/') {
			std::fs::create_dir_all(&outpath)?;
		} else {
			if let Some(parent) = outpath.parent() {
				std::fs::create_dir_all(parent)?;
			}
			let mut outfile = File::create(&outpath)?;
			std::io::copy(&mut file, &mut outfile)?;
		}
	}
	Ok(())
}

pub fn group_files_by_student(
	dir: &Path,
	students: &[Student],
) -> Result<(), Box<dyn std::error::Error>> {
	let mut moved = 0;
	let mut skipped = 0;

	for entry in dir.read_dir()? {
		let entry = match entry {
			Ok(e) => e,
			Err(e) => {
				warn!("Failed to read entry: {}", e);
				skipped += 1;
				continue;
			}
		};

		let path = entry.path();

		// ✅ 只处理普通文件，跳过目录
		if !entry.file_type()?.is_file() {
			continue;
		}

		let file_name = match path.file_name().and_then(|n| n.to_str()) {
			Some(name) => name,
			None => {
				warn!("Skipping non-UTF8 filename: {:?}", path);
				skipped += 1;
				continue;
			}
		};

		let parts: Vec<&str> = file_name.splitn(2, '_').collect();
		if parts.len() < 2 {
			warn!("Invalid filename format: {}", file_name);
			skipped += 1;
			continue;
		}
		let student_id = parts[0];

		let student = match students.iter().find(|s| s.sis_login_id == student_id) {
			Some(s) => s,
			None => {
				warn!("Student ID {} not found in roster", student_id);
				skipped += 1;
				continue;
			}
		};

		let student_folder = dir.join(&student_id);
		if !student_folder.exists() {
			info!("Creating folder: {}", student_folder.display());
			fs::create_dir_all(&student_folder)?;
		}

		let target_path = student_folder.join(file_name);

		if target_path.exists() {
			warn!("File {} already exists, overwriting", target_path.display());
			fs::remove_file(&target_path)?;
		}

		info!(
			"Moving file: {} → {}",
			path.display(),
			target_path.display()
		);

		fs::rename(&path, &target_path)?;
		moved += 1;
	}

	info!("Grouped files complete: {} moved, {} skipped", moved, skipped);
	Ok(())
}