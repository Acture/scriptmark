use std::any::Any;
use std::collections::HashMap;
use suite::test_suite::{TestSuite, TestSuiteTrait};

pub mod circle_area;
pub mod population;

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

pub fn get_solution_map() -> HashMap<TestSuiteType, Box<dyn TestSuiteTrait>>
where
{
    let mut function_map = HashMap::new();
    function_map.insert(TestSuiteType::CircleArea, circle_area::get_test_suite());
    function_map.insert(TestSuiteType::Population, population::get_test_suite());

    function_map
}
