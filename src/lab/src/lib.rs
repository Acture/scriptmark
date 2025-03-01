use lazy_static::lazy_static;
use std::collections::HashMap;
use suite::test_suite::TestSuiteObject;

pub mod circle_area;
pub mod population;
mod sequence;

#[derive(Eq, PartialEq, Hash)]
pub enum TestSuiteType {
	CircleArea,
	Population,
	Sequence,
}

impl TestSuiteType {
	pub fn from_str(s: &str) -> TestSuiteType {
		match s {
			"circle_area" => TestSuiteType::CircleArea,
			"population" => TestSuiteType::Population,
			"sequence" => TestSuiteType::Sequence,
			_ => panic!("Invalid test suite type: {}", s),
		}
	}

	pub fn from_endwith(s: &str) -> TestSuiteType {
		for possible in ["circle_area", "population", "sequence"].iter() {
			if s.ends_with(possible) {
				return TestSuiteType::from_str(possible);
			}
		}
		panic!("Invalid test suite type: {}", s)
	}

	pub fn to_string(&self) -> String {
		match self {
			TestSuiteType::CircleArea => "circle_area".to_string(),
			TestSuiteType::Population => "population".to_string(),
			TestSuiteType::Sequence => "sequence".to_string(),
		}
	}
}

lazy_static! {
	pub static ref TEST_SUITE_MAP: HashMap<TestSuiteType, Box<dyn TestSuiteObject>> =
		HashMap::from([
			(
				TestSuiteType::CircleArea,
				Box::new(circle_area::CIRCLE_AREA_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Population,
				Box::new(population::POPULATION_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Sequence,
				Box::new(sequence::SEQUENCE_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
		])
	;
}
