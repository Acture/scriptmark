pub mod solutions;

pub fn add(left: u64, right: u64) -> u64 {
	left + right
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::error::Error;

	#[test]
	fn test_valid_email() -> Result<(), Box<dyn Error>> {
		let test_suite = solutions::TEST_SUITES.iter().find(
			|dts| dts.get_name() == "valid_email"
		).expect("Test suite not found");

		test_suite.run_file()

		Ok(())
	}
}
