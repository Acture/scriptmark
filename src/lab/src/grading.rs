use std::collections::HashMap;
use suite::define_test_suite;

type InputType = Vec<i64>;
type OutputType = Vec<String>;
fn generate_inputs() -> InputType {
    util::generate(42, 15, 60, 100)
        .into_iter()
        .chain(util::generate(42, 3, 0, 60))
        .chain(vec![123])
        .chain(vec![-123])
        .collect()
}

fn get_answers() -> OutputType {
    generate_inputs()
        .iter()
        .map(|&score| {
            match score {
                90..=100 => "A", // 包含90和100
                85..90 => "A-",
                82..85 => "B+",
                78..82 => "B",
                75..78 => "B-",
                71..75 => "C+",
                66..71 => "C",
                62..66 => "C-",
                60..62 => "D",
                0..60 => "F",
                _ => "Error!",
            }
            .to_string()
        })
        .collect()
}

fn runner_fn(path: &std::path::Path) -> OutputType {
    if !path.exists() {
        panic!("Test file not found: {}", path.display());
    }
    let content = std::fs::read_to_string(path).expect("Failed to read file");
    let inputs = generate_inputs();
    inputs
        .iter()
        .map(|&score| {
            match runner::python::run_code::<String>(
                content.clone(),
                Some(format!("{}\n", score)),
                None::<&[String]>,
            ) {
                Ok(output) => output,
                Err(e) => format!("Failed to run code: {:?}", e),
            }
        })
        .collect()
}

fn judge_fn(result: &OutputType, expected: &OutputType) -> Vec<suite::test_suite::TestResult> {
    result
        .iter()
        .zip(expected.iter())
        .map(|(result, expected)| {
            let mut res = suite::test_suite::TestResult::builder().build();
            let res_letter_o = result.lines().next();
            let expected_letter_o = expected.lines().next();
            let (res_letter, expected_letter) = match (res_letter_o, expected_letter_o) {
                (Some(res_letter), Some(expected_letter)) => (res_letter, expected_letter),
                _ => {
                    res.passed = false;
                    res.infos.get_or_insert_with(HashMap::new).insert(
                        "Failed to Parse".to_string(),
                        format!("Result: <{:?}> Expected: <{:?}>", result, expected,),
                    );
                    return res;
                }
            };
            if !res_letter.ends_with(expected_letter) {
                res.passed = false;
                res.infos.get_or_insert_with(HashMap::new).insert(
                    "Wrong Result".to_string(),
                    format!("Expected result: <{:?}> Got <{:?}>", expected, result,),
                );
            } else {
                res.passed = true;
            }
            res
        })
        .collect()
}

define_test_suite!(
    pub name = GRADING_TEST_SUITE,
    inputs = {
        type = InputType,
        init = generate_inputs(),
        clone = |x: & InputType| x.clone()
    },
    answers = {
        type = OutputType,
        init = get_answers(),
        clone = |x: &OutputType| x.clone()
    },
    runner = runner_fn,
    judge = judge_fn
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_inputs() {
        let inputs = generate_inputs();
        println!("Generated inputs: {:?}", inputs);
    }

    #[test]
    fn test_get_answers() {
        let inputs = generate_inputs();
        let answers = get_answers();

        for (input, answer) in inputs.iter().zip(answers.iter()) {
            println!("Input: {}, Answer: {}", input, answer)
        }
    }

    #[test]
    fn test_runner_func() {
        let path = std::path::Path::new("src/solutions/grading.py");
        let res = runner_fn(&path);
        println!("Run results: {:?}", res);
    }

    #[test]
    fn test_judge_func() {
        let path = std::path::Path::new("src/solutions/grading.py");
        let res = runner_fn(&path);
        let answers = get_answers();
        let inputs = generate_inputs();
        let judge_result = judge_fn(&res, &answers);
        for (index, r) in judge_result.iter().enumerate() {
            if !r.passed {
                println!("Failed test case: {}", index);
                println!("Input: {}", inputs[index]);
                println!("Result: {:?}", r);
                println!("Expected: {:?}", answers[index]);
                println!("Output: {:?}", res[index]);
                assert!(false);
            }
        }
    }
}
