use argon2::{Algorithm, Argon2, Params, Version, password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng}};

use crate::{Error, Result};

fn engine() -> Result<Argon2<'static>> {
    let params = Params::new(19456, 2, 1, None).map_err(|_| Error::Crypto)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

pub async fn hash(password: String) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        engine()?.hash_password(password.as_bytes(), &salt).map(|hash| hash.to_string()).map_err(|_| Error::Crypto)
    }).await.map_err(|_| Error::Crypto)?
}

pub async fn verify(password: String, hash: String) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        let hash = PasswordHash::new(&hash).map_err(|_| Error::Crypto)?;
        Ok(engine()?.verify_password(password.as_bytes(), &hash).is_ok())
    }).await.map_err(|_| Error::Crypto)?
}

pub fn validate(password: &str) -> Result<()> {
    if !(12..=128).contains(&password.len()) {
        return Err(Error::Validation("password must contain 12-128 UTF-8 bytes".into()));
    }
    Ok(())
}