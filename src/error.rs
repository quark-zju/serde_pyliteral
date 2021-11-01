use std::borrow::Cow;
use std::fmt;
use std::num::ParseFloatError;
use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Generic(String),

    #[error("expect {0}, got {1}")]
    TypeMismatch(&'static str, Cow<'static, str>),

    #[error(transparent)]
    ParseInt(#[from] ParseIntError),

    #[error(transparent)]
    ParseFloat(#[from] ParseFloatError),

    #[error("cannot parse string: {0}")]
    ParseString(Cow<'static, str>),

    #[error("cannot parse bytes: {0}")]
    ParseBytes(Cow<'static, str>),

    #[error("cannot auto-detect type: {0:?}")]
    ParseAny(String),

    #[error("cannot serialize nan")]
    NaN,

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
