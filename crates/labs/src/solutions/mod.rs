use crate::solutions::strip_html_tags::STRIP_HTML_TAG_TESTSUITE;
use crate::solutions::valid_email::VALID_EMAIL_TESTSUITE;
use common::traits::testsuite::DynTestSuite;
use std::ops::Deref;
use std::sync::LazyLock;

pub mod valid_email;
mod strip_html_tags;

pub static TEST_SUITES: LazyLock<[&'static dyn DynTestSuite; 2]> = LazyLock::new(|| {
	[
		VALID_EMAIL_TESTSUITE.deref(),
		STRIP_HTML_TAG_TESTSUITE.deref()
	]
});