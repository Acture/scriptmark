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

		let test_path = dev::env::DATA_DIR.clone()
			.join("COMP110042.09/作业8 (104838)/23307110287/23307110287_232949_5641683_Lab8_1.py");

		let res = test_suite.pipelined(&test_path);

		print!("{:?}", res);

		Ok(())
	}
}
