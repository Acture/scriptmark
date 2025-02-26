pub trait TestDataProvider<T> {
    fn generate(&self) -> T;
}

pub trait TestRunner<T> {
    fn run(&self, data: &T);
}
pub trait TestJudger<T> {
    fn judge(&self, data: &T) -> bool;
}

pub struct TestSuite<T> {
    data_provider: Box<dyn TestDataProvider<T>>,
    runner: Box<dyn TestRunner<T>>,
    judger: Box<dyn TestJudger<T>>,
}

impl<T> TestSuite<T> {
    pub fn new(
        data_provider: Box<dyn TestDataProvider<T>>,
        runner: Box<dyn TestRunner<T>>,
        judger: Box<dyn TestJudger<T>>,
    ) -> Self {
        Self {
            data_provider,
            runner,
            judger,
        }
    }

    pub fn run_test(&self) -> bool {
        let test_data = self.data_provider.generate();
        self.runner.run(&test_data);
        self.judger.judge(&test_data)
    }

    pub fn run_multiple(&self, count: usize) -> (usize, usize) {
        let mut passed = 0;

        for _ in 0..count {
            if self.run_test() {
                passed += 1;
            }
        }

        (passed, count)
    }
}
