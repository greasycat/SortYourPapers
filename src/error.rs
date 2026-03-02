use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("missing required configuration: {0}")]
    MissingConfig(&'static str),

    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),

    #[error("walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("pdf extraction error: {0}")]
    Pdf(String),

    #[error("llm error: {0}")]
    Llm(String),

    #[error("execution error: {0}")]
    Execution(String),
}
