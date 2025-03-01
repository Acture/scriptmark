use lazy_static::lazy_static;
use std::collections::HashMap;
use suite::test_suite::{TestSuiteObject};

pub mod circle_area;
pub mod population;
mod sequence;

#[derive(Eq, PartialEq, Hash)]
pub enum TestSuiteType {
	CircleArea,
	Population,
}

impl TestSuiteType {
	pub fn from_str(s: &str) -> TestSuiteType {
		match s {
			"circle_area" => TestSuiteType::CircleArea,
			"population" => TestSuiteType::Population,
			_ => panic!("Invalid test suite type: {}", s),
		}
	}

	pub fn from_endwith(s: &str) -> TestSuiteType {
		if s.ends_with("circle_area") {
			TestSuiteType::CircleArea
		} else if s.ends_with("population") {
			TestSuiteType::Population
		} else {
			panic!("Invalid test suite type: {}", s)
		}
	}

	pub fn to_string(&self) -> String {
		match self {
			TestSuiteType::CircleArea => "circle_area".to_string(),
			TestSuiteType::Population => "population".to_string(),
		}
	}
}

lazy_static! {
	pub static ref TEST_SUITE_MAP: HashMap<TestSuiteType, Box<dyn TestSuiteObject>> = {
		let mut function_map = HashMap::new();
		function_map.insert(
			TestSuiteType::CircleArea,
			Box::new(circle_area::CIRCLE_AREA_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
		);
		function_map.insert(
			TestSuiteType::Population,
			Box::new(population::POPULATION_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
		);
		function_map
	};
}
