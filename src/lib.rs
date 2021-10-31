pub mod de;
pub mod error;
mod peek;
pub mod ser;
mod unicode;

#[cfg(test)]
mod tests;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;

pub use ser::to_string;
pub use ser::to_string_pretty;
pub use ser::to_vec;
pub use ser::to_vec_pretty;
pub use ser::to_writer;
pub use ser::to_writer_pretty;
