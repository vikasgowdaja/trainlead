pub mod audit;
pub mod auth;
pub mod clock;
pub mod db;
mod password;
mod sessions;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    Validation(String),
    #[error("invalid credentials")]
    Unauthorized,
    #[error("permission denied")]
    Forbidden,
    #[error("too many requests")]
    RateLimited,
    #[error("authentication provider is not configured")]
    ProviderUnavailable,
    #[error("invalid server configuration")]
    Configuration,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("cryptographic operation failed")]
    Crypto,
}

pub type Result<T> = std::result::Result<T, Error>;