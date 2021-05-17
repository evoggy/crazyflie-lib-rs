use std::array::TryFromSliceError;

#[derive(Debug)]
pub enum Error {
    ProtocolError(String),
    ParamError(String),
    ConversionError(TryFromSliceError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Foo {}", self))
    }
}

impl std::error::Error for Error {}

impl From<TryFromSliceError> for Error {
    fn from(e: TryFromSliceError) -> Self {
        Self::ConversionError(e)
    }
}