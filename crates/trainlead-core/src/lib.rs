pub mod dedup;
pub mod extraction;
pub mod models;
pub mod parser;
pub mod pipeline;
pub mod rules;
pub mod source;
pub mod store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Validation(String),
    #[error("export parse error at line {line}: {detail}")]
    Export { line: usize, detail: String },
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SQLite error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("LLM request failed with HTTP status {0}")]
    ApiStatus(u16),
    #[error("missing configuration: {0}")]
    Configuration(String),
    #[error("source I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
