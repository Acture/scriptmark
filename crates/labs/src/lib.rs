use common::defines::testsuite::TestSuiteObject;
use lazy_static::lazy_static;
use std::collections::HashMap;
use strum::IntoEnumIterator;
use strum_macros::{AsRefStr, Display, EnumIter, EnumString};


mod solutions;
mod test_suites;

#[derive(Debug, Eq, PartialEq, Hash, EnumString, Display, EnumIter, AsRefStr)]
pub enum TestSuiteType {
	#[strum(serialize = "circle_area")]
	CircleArea,
	#[strum(serialize = "population")]
	Population,
	#[strum(serialize = "sequence")]
	Sequence,
	#[strum(serialize = "three_number")]
	ThreeNumber,
	#[strum(serialize = "circle_area_2")]
	CircleArea2,
	#[strum(serialize = "chicken_rabbit")]
	ChickenRabbit,
	#[strum(serialize = "poem")]
	Poem,
	#[strum(serialize = "bigger")]
	Bigger,
	#[strum(serialize = "grading")]
	Grading,
}

impl TestSuiteType {
	pub fn from_endwith(s: &str) -> Option<TestSuiteType> {
		TestSuiteType::iter().find(|variant| s.ends_with(variant.as_ref()))
	}
}

lazy_static! {
	pub static ref TEST_SUITE_MAP: HashMap<TestSuiteType, Box<dyn TestSuiteObject>> =
		HashMap::from([
			(
				TestSuiteType::CircleArea,
				Box::new(solutions::circle_area::CIRCLE_AREA_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Population,
				Box::new(solutions::population::POPULATION_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Sequence,
				Box::new(solutions::sequence::SEQUENCE_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::ThreeNumber,
				Box::new(solutions::three_number::THREE_NUMBER_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::CircleArea2,
				Box::new(solutions::circle_area_2::CIRCLE_AREA_TEST_2_SUITE.clone())
					as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::ChickenRabbit,
				Box::new(solutions::chicken_rabbit::CHICKEN_RABBIT_TEST_SUITE.clone())
					as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Poem,
				Box::new(solutions::poem::POEM_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
			),
			(
				TestSuiteType::Bigger,
				Box::new(solutions::bigger::BIGGER_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>
			),
			(
				TestSuiteType::Grading,
				Box::new(solutions::grading::GRADING_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>
			)
		]);
}
