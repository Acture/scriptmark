use crate::defines::assignment::Assignment;
use crate::defines::student::Student;
use crate::defines::task::Task;
use crate::defines::testresult::TestResult;
use derivative::Derivative;
use displaydoc::Display;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Submission {
	#[builder(default, setter(strip_option, into))]
	#[derivative(Hash = "ignore")]
	pub score: Option<f64>,
	#[builder(default, setter(strip_option, into))]
	pub submission_path: Option<PathBuf>,
	#[builder(default, setter(strip_option, into))]
	#[derivative(Hash = "ignore")]
	pub cached_hash: Option<u64>,
	#[builder(default, setter(into))]
	pub test_result: Vec<TestResult>,

	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub belong_to_student: Weak<RefCell<Student>>,

	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub belong_to_task: Weak<RefCell<Task>>,

	#[serde(skip)]
	#[builder(default, setter(into))]
	#[derivative(PartialEq = "ignore", Hash = "ignore")]
	pub belong_to_assignment: Weak<RefCell<Assignment>>,
}

impl Submission {
	pub fn to_serializable(&self) -> SerializableSubmission {
		SerializableSubmission {
			score: self.score,
			submission_path: self.submission_path.clone(),
			cached_hash: self.cached_hash,
			test_result: self.test_result.clone(),
			belong_to_student_sis_id: match self.belong_to_task.upgrade() {
				Some(task) => Some(task.borrow().name.clone()),
				None => None
			},
			belong_to_task_name: match self.belong_to_student.upgrade() {
				Some(student) => Some(student.borrow().sis_login_id.clone()),
				None => None
			},
			belong_to_assignment_name: match self.belong_to_assignment.upgrade() {
				Some(assignment) => Some(assignment.borrow().name.clone()),
				None => None
			}
		}
	}

	pub fn from_serializable(serializable: SerializableSubmission, students: &[Rc<RefCell<Student>>], tasks: &[Rc<RefCell<Task>>]) -> Self {
		let belong_student = serializable.belong_to_student_sis_id
			.and_then(|student_sis_id| students.iter().find(|student| student.borrow().sis_login_id == student_sis_id));
		let belong_task = serializable.belong_to_task_name
			.and_then(|task_name| tasks.iter().find(|task| task.borrow().name == task_name));
		Self {
			score: serializable.score,
			submission_path: serializable.submission_path,
			cached_hash: serializable.cached_hash,
			test_result: serializable.test_result,
			belong_to_student: match belong_student {
				Some(student) => Rc::downgrade(student),
				None => Weak::new(),
			},
			belong_to_task: match belong_task {
				Some(task) => Rc::downgrade(task),
				None => Weak::new(),
			},
			belong_to_assignment: match belong_task {
				Some(task) => task.borrow().belong_to_assignment.clone(),
				None => Weak::new(),
			}
		}
	}
}

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct SerializableSubmission {
	#[builder(default, setter(strip_option, into))]
	#[derivative(Hash = "ignore")]
	pub score: Option<f64>,
	#[builder(default, setter(strip_option, into))]
	pub submission_path: Option<PathBuf>,
	#[builder(default, setter(strip_option, into))]
	#[derivative(Hash = "ignore")]
	pub cached_hash: Option<u64>,
	#[builder(default, setter(into))]
	pub test_result: Vec<TestResult>,

	#[builder(default, setter(strip_option, into))]
	pub belong_to_student_sis_id: Option<String>,
	#[builder(default, setter(strip_option, into))]
	pub belong_to_task_name: Option<String>,
	#[builder(default, setter(strip_option, into))]
	pub belong_to_assignment_name: Option<String>,
}

impl Submission {
	pub fn update_hash(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		if let Some(path) = &self.submission_path {
			if !path.exists() {
				panic!("Submission path does not exist: {}", path.display());
			}
			let contents = std::fs::read(path)?;
			let mut hasher = DefaultHasher::new();
			contents.hash(&mut hasher);
			let hash = hasher.finish();
			self.cached_hash = Some(hash);
			Ok(())
		} else {
			Err("No submission path".into())
		}
	}
}
