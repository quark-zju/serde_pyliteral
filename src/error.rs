use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Generic(String),

    #[error("{0} is not supported")]
    Unsupported(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Generic(msg.to_string())
    }
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Generic(msg.to_string())
    }
}

pub(crate) fn unsupported<T>(message: &'static str) -> Result<T, Error> {
    Err(Error::Unsupported(message))
}
