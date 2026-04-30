//! Error types for srt-core. Filled in Task 2 + extended in Task 8.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("placeholder; replaced in Task 2")]
    Placeholder,
}

pub type Result<T> = std::result::Result<T, Error>;
