pub type BoxedErr = Box<dyn std::error::Error>;
pub type ResultWithStdErr<T> = Result<T, BoxedErr>;