use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use suite::test_suite::{TestSuite, TestSuiteTrait};

pub mod circle_area;
pub mod population;

#[derive(Eq, PartialEq, Hash)]
pub enum TestSuiteType {
	Population,
}

pub fn get_function_map() -> HashMap<TestSuiteType, Box<TestSuite>>
where
{
	let mut function_map = HashMap::new();
	function_map.insert(
		TestSuiteType::Population,
		Box::new(population::get_test_suite())
	);
	function_map
}