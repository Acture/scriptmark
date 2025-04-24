use crate::defines::class;
use crate::defines::student::Student;
use std::collections::HashMap;
use suite::test_suite::TestResult;

pub fn check_assignment(
	selected_class: &class::Class,
	selected_assignment_name: &str,
	allow_custom: bool,
) -> (
	HashMap<Student, Vec<TestResult>>,
	HashMap<u64, Vec<Student>>,
) {
	unimplemented!()
}
