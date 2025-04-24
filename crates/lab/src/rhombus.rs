use itertools::chain;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use suite::define_test_suite;
use suite::test_suite::TestResult;

fn get_rhombus<T: Into<usize>>(radius: T, symbol: char, padding_symbol: char) -> String {
    let r = radius.into();
    let max = r * 2;
    (0..=max).fold(String::new(), |acc, i| {
        let layer = if i <= r { i } else { max - i };
        let symbols = (layer * 2 + 1) as usize;
        let pad = (r - layer) as usize;

        let line = format!(
            "{}{}{}\n",
            std::iter::repeat(padding_symbol)
                .take(pad)
                .collect::<String>(),
            std::iter::repeat(symbol).take(symbols).collect::<String>(),
            std::iter::repeat(padding_symbol)
                .take(pad)
                .collect::<String>(),
        );
        acc + &line
    })
}

type INPUT_TYPE = Vec<i64>;
type OUTPUT_TYPE = Vec<String>;

fn get_inputs() -> INPUT_TYPE {
    util::generate(42, 10, 1, 20)
}

fn get_answer_rhombus(radius: usize) -> String {
    get_rhombus(radius, '*', ' ')
}

fn get_answers() -> Vec<String> {
    std::iter::once("请输入n：".to_string())
        .chain(get_inputs().iter().map(|&r| get_answer_rhombus(r as usize)))
        .collect()
}
fn runner_fn(path: &Path) -> OUTPUT_TYPE {
    if !path.exists() {
        panic!("Test file not found: {}", path.display());
    }
    let content = fs::read_to_string(path).expect("Failed to read file");
    let inputs = get_inputs();
    inputs
        .iter()
        .map(|input| {
            runner::python::run_code::<String>(
                content.clone(),
                Some(input.to_string()),
                None::<&[String]>,
            )
            .unwrap_or_else(|e| format!("Failed to run code: {:?}", e))
        })
        .collect::<Vec<_>>()
}

fn reverse_string(s: &str) -> String {
    s.chars().rev().collect::<String>()
}

fn judge_fn(_results: &OUTPUT_TYPE, _answers: &OUTPUT_TYPE) -> Vec<suite::test_suite::TestResult> {
    _results
        .iter()
        .zip(_answers.iter())
        .map(|(result, answer)| {
            let mut test_result = TestResult::builder().passed(true).build();

            result.lines().zip(answer.lines()).enumerate().for_each(
                |(i, (result_line, answer_line))| {
                    let reversed_answer = reverse_string(answer_line);
                    if result_line != answer_line
                        && result_line != reversed_answer
                        && !answer_line.starts_with(result_line)
                        && !reversed_answer.starts_with(result_line)
                    {
                        if i == 0 {
                            return;
                        }
                        test_result.passed = false;
                        test_result.infos = Some(HashMap::from([(
                            format!("Difference in Line {}", i),
                            format!("Given: {} != Expected: {}", result, answer),
                        )]));
                    }
                },
            );
            test_result
        })
        .collect()
}

define_test_suite!(
    pub name = RHOMBUS,
    inputs = {
        type = INPUT_TYPE,
        init = get_inputs(),
        clone = |x: &INPUT_TYPE| x.clone()
    },
    answers = {
        type = OUTPUT_TYPE,
        init = get_answers(),
        clone = |x: &OUTPUT_TYPE| x.clone()
    },
    runner = runner_fn,
    judge = judge_fn
);

#[cfg(test)]
mod tests {
    use super::*;
    use itertools::all;

    #[test]
    fn test_get_rhombus() {
        let r = get_rhombus(5_usize, '*', ' ');
        println!("{}", r);
    }

    #[test]
    fn test_get_answers() {
        let answers = get_answers();
        for answer in answers {
            println!("{}", answer);
        }
    }

    #[test]
    fn test_runner_fn() {
        let path = std::env::current_dir().unwrap();
        println!("{}", path.display());
        let inputs = get_inputs();
        let answers = get_answers();
        for (input, answer) in inputs.iter().zip(answers.iter()) {
            let result = runner_fn(&Path::new(
                "../../data/COMP110042_09/lab6_rhombus/20300120010_145502_5566366_lab6.1.py",
            ));
            println!("input: {}, answer: {}", input, answer);
            println!("result: {}", result[0]);
        }
    }

    #[test]
    fn test_judge_fn() {
        let inputs = get_inputs();
        let answers = get_answers();
        let test_examples = runner_fn(&Path::new(
            "../../data/COMP110042_09/lab6_rhombus/23300120006_231161_5570103_Lab6_1.py",
        ));
        let result = judge_fn(&test_examples, &answers);

        inputs
            .iter()
            .zip(result.iter())
            .filter(|(_, test_result)| !test_result.passed)
            .for_each(|(input, test_result)| {
                println!("input: {}, result: {:?}", input, test_result);
            });
    }
}
