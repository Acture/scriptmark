use crate::defines::assignment::{Assignment, SerializableAssignment};
use crate::defines::class::{Class, SerializableClass};
use crate::defines::student::{SerializableStudent, Student};
use crate::defines::submission::{SerializableSubmission, Submission};
use crate::defines::task::{SerializableTask, Task};
use crate::rc_ref;
use derivative::Derivative;
use displaydoc::Display;
use log::warn;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::rc::Rc;
use typed_builder::TypedBuilder;

#[derive(TypedBuilder, Derivative, Serialize, Deserialize, Display)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct FullDump {
	classes: Vec<SerializableClass>,
	students: Vec<SerializableStudent>,
	assignments: Vec<SerializableAssignment>,
	tasks: Vec<SerializableTask>,
	submissions: Vec<SerializableSubmission>,
}

pub fn save_dump(classes: &[Rc<RefCell<Class>>], path: &Path) -> Result<(), Box<dyn std::error::Error>> {
	let assignments: Vec<Assignment> = classes
		.iter()
		.flat_map(|c| c.borrow().assignments.iter().map(|a| a.borrow().clone()).collect::<Vec<_>>())
		.collect();

	let students: Vec<Student> = classes
		.iter()
		.flat_map(|c| c.borrow().students.iter().map(|s| s.borrow().clone()).collect::<Vec<_>>())
		.collect();

	let submissions: Vec<Submission> = classes
		.iter()
		.flat_map(|c| c.borrow().students.iter().flat_map(|s| s.borrow().submissions.iter().map(|s| s.borrow().clone()).collect::<Vec<_>>()).collect::<Vec<_>>())
		.collect();

	let tasks: Vec<Task> = assignments
		.iter()
		.flat_map(|a| a.tasks.iter().map(|t| t.borrow().clone()))
		.collect();

	let dump = FullDump {
		classes: classes.iter().map(|c| c.borrow().clone().to_serializable()).collect(),
		assignments: assignments.iter().map(|a| a.to_serializable()).collect(),
		students: students.iter().map(|s| s.to_serializable()).collect(),
		submissions: submissions.iter().map(|s| s.to_serializable()).collect(),
		tasks: tasks.iter().map(|t| t.to_serializable()).collect(),
	};

	let file = File::create(path)?;
	let writer = BufWriter::new(file);
	serde_json::to_writer_pretty(writer, &dump)?;

	Ok(())
}

pub fn load_dump(path: &Path) -> Result<Vec<Rc<RefCell<Class>>>, Box<dyn std::error::Error>> {
	let file = File::open(path)?;
	let reader = BufReader::new(file);
	let dump: FullDump = serde_json::from_reader(reader)?;

	let classes = dump.classes.iter().map(
		|sc| rc_ref!(Class::from_serializable(sc.clone()))
	).collect::<Vec<_>>();

	let students = dump.students.iter().map(
		|ss| rc_ref!(Student::from_serializable(ss.clone(), &classes))
	).collect::<Vec<_>>();

	let assignments = dump.assignments.iter().map(
		|sa| rc_ref!(Assignment::from_serializable(sa.clone(), &classes))
	).collect::<Vec<_>>();

	let tasks = dump.tasks.iter().map(
		|st| rc_ref!(Task::from_serializable(st.clone(), &assignments))
	).collect::<Vec<_>>();

	let submissions = dump.submissions.iter().map(
		|ss| rc_ref!(Submission::from_serializable(ss.clone(), &students, &tasks))
	).collect::<Vec<_>>();

	for submission in submissions.iter() {
		match submission.borrow().belong_to_task.upgrade() {
			Some(task) => task.borrow_mut().submissions.push(submission.clone()),
			None => warn!("Task not found for submission"),
		}
		match submission.borrow().belong_to_student.upgrade() {
			Some(student) => student.borrow_mut().submissions.push(submission.clone()),
			None => warn!("Student not found for submission"),
		}
	}

	for task in tasks.iter() {
		match task.borrow().belong_to_assignment.upgrade() {
			Some(assignment) => assignment.borrow_mut().tasks.push(task.clone()),
			None => warn!("Assignment not found for task"),
		}
	}

	for assignment in assignments.iter() {
		match assignment.borrow().belong_to_class.upgrade() {
			Some(class) => class.borrow_mut().assignments.push(assignment.clone()),
			None => warn!("Class not found for assignment"),
		}
	}

	for student in students.iter() {
		match student.borrow().belong_to_class.upgrade() {
			Some(class) => class.borrow_mut().students.push(student.clone()),
			None => warn!("Class not found for student"),
		}
	}


	Ok(classes)
}
