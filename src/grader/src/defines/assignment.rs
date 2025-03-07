use crate::defines::student::Student;
use std::collections::HashMap;
use std::path::PathBuf;
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub struct Assignment {
    pub name: String,
    pub path: PathBuf,
}

impl Assignment {
    pub fn group_by_student(&self, students: &[Student]) -> HashMap<String, Vec<PathBuf>> {
        let mut rec: HashMap<String, Vec<PathBuf>> = students
            .iter()
            .map(|student| (student.sis_login_id.to_string(), vec![]))
            .collect();

        for entry in self.path.read_dir().expect("read_dir call failed") {
            let entry = entry.expect("entry failed");
            let path = entry.path();

            if path.is_dir() {
                let id = path
                    .file_stem()
                    .expect("file_stem failed")
                    .to_string_lossy()
                    .to_string();
                path.read_dir()
                    .expect("read_dir call failed")
                    .for_each(|entry| {
                        let entry = entry.expect("entry failed");
                        let path = entry.path();
                        if path.extension() == Some("py".as_ref()) {
                            rec.entry(id.clone()).or_default().push(path);
                        }
                    });
            } else if path.is_file() && path.extension() == Some("py".as_ref()) {
                let file_name = path
                    .file_name()
                    .expect("file_name failed")
                    .to_string_lossy()
                    .to_string();
                let id = file_name
                    .split('_')
                    .next()
                    .expect("split failed")
                    .to_string();
                rec.entry(id.clone()).or_default().push(path);
            }
        }

        rec.iter()
            .map(|(id, file_paths)| {
                let dir = self.path.join(id);
                if !dir.exists() {
                    std::fs::create_dir(&dir).expect("create_dir failed");
                }
                let new_paths: Vec<PathBuf> = file_paths
                    .iter()
                    .map(|file_path| match file_path.strip_prefix(&dir) {
                        Ok(_) => file_path.to_path_buf(),
                        Err(_) => {
                            let new_path =
                                dir.join(file_path.file_name().expect("file_name failed"));
                            std::fs::rename(file_path, &new_path).expect("rename failed");
                            new_path
                        }
                    })
                    .collect();
                (id.clone(), new_paths)
            })
            .collect()
    }
}
