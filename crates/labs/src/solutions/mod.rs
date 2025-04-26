use crate::solutions::valid_email::VALID_EMAIL_TESTSUITE;
use common::traits::testsuite::DynTestSuite;
use std::iter::Iterator;
use std::ops::Deref;
use std::sync::LazyLock;

pub mod valid_email;


pub static TEST_SUITES: LazyLock<[&'static dyn DynTestSuite; 1]> = LazyLock::new(|| {
	[
		VALID_EMAIL_TESTSUITE.deref(),
	]
});