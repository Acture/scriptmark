use crate::solutions::lower_case_to_upper_case::LOWER_CASE_TO_UPPER_CASE_TESTSUITE;
use crate::solutions::strip_html_tags::STRIP_HTML_TAG_TESTSUITE;
use crate::solutions::valid_email::VALID_EMAIL_TESTSUITE;
use common::traits::testsuite::DynTestSuite;
use std::ops::Deref;
use std::sync::LazyLock;

pub mod valid_email;
mod strip_html_tags;
mod lower_case_to_upper_case;

pub static TEST_SUITES: LazyLock<[&'static dyn DynTestSuite; 3]> = LazyLock::new(|| {
	[
		VALID_EMAIL_TESTSUITE.deref(),
		STRIP_HTML_TAG_TESTSUITE.deref(),
		LOWER_CASE_TO_UPPER_CASE_TESTSUITE.deref(),
	]
});