use lazy_static::lazy_static;
use std::collections::HashMap;
use strum::IntoEnumIterator;
use strum_macros::{AsRefStr, Display, EnumIter, EnumString};
use suite::test_suite::TestSuiteObject;

mod bigger;
mod chicken_rabbit;
mod circle_area;
mod circle_area_2;
mod grading;
mod hundred_chickens;
mod k_number;
mod poem;
mod population;
mod rhombus;
mod sequence;
mod three_number;

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
            (
                TestSuiteType::ThreeNumber,
                Box::new(three_number::THREE_NUMBER_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
            ),
            (
                TestSuiteType::CircleArea2,
                Box::new(circle_area_2::CIRCLE_AREA_TEST_2_SUITE.clone())
                    as Box<dyn TestSuiteObject>,
            ),
            (
                TestSuiteType::ChickenRabbit,
                Box::new(chicken_rabbit::CHICKEN_RABBIT_TEST_SUITE.clone())
                    as Box<dyn TestSuiteObject>,
            ),
            (
                TestSuiteType::Poem,
                Box::new(poem::POEM_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>,
            ),
            (
                TestSuiteType::Bigger,
                Box::new(bigger::BIGGER_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>
            ),
            (
                TestSuiteType::Grading,
                Box::new(grading::GRADING_TEST_SUITE.clone()) as Box<dyn TestSuiteObject>
            )
        ]);
}
