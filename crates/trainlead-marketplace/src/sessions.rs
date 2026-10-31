use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction, Row};
use uuid::Uuid;

use crate::{Error, Result, audit, auth::{Actor, AuthService, Tokens}};

const ISSUER: &str = "trainlead";
const AUDIENCE: &str = "trainlead-api";
const ACCESS_SECONDS: i64 = 900;
const REFRESH_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
struct Claims { sub: String, sid: String, iss: String, aud: String, iat: i64, exp: i64 }

pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn token_hash(token: &str) -> String { hex::encode(Sha256::digest(token.as_bytes())) }

impl AuthService {
    pub(crate) async fn issue_session(&self, transaction: &mut Transaction<'_, Sqlite>, user: &str, family: &str) -> Result<Tokens> {
        let id = Uuid::new_v4().to_string();
        let now = self.clock.now();
        let refresh_token = random_token();
        sqlx::query("INSERT INTO sessions(id,family_id,user_id,refresh_hash,created_at,access_expires_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)")
            .bind(&id).bind(family).bind(user).bind(token_hash(&refresh_token)).bind(now).bind(now + ACCESS_SECONDS).bind(now + REFRESH_SECONDS)
            .execute(&mut **transaction).await?;
        let access_token = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &Claims {
            sub: user.into(), sid: id, iss: ISSUER.into(), aud: AUDIENCE.into(), iat: now, exp: now + ACCESS_SECONDS,
        }, &EncodingKey::from_secret(&self.secret)).map_err(|_| Error::Crypto)?;
        Ok(Tokens { access_token, refresh_token, token_type: "Bearer".into(), expires_in: ACCESS_SECONDS as u32 })
    }

    pub async fn authenticate(&self, token: &str) -> Result<Actor> {
        if token.len() > 4096 { return Err(Error::Unauthorized); }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[ISSUER]); validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "sub", "iat", "iss", "aud"]);
        validation.validate_exp = false;
        let claims = jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(&self.secret), &validation)
            .map_err(|_| Error::Unauthorized)?.claims;
        let now = self.clock.now();
        if claims.exp <= now || claims.iat > now + 60 || claims.exp - claims.iat != ACCESS_SECONDS { return Err(Error::Unauthorized); }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions s JOIN users u ON u.id=s.user_id
            WHERE s.id=?1 AND s.user_id=?2 AND s.revoked_at IS NULL AND s.rotated_at IS NULL
            AND s.access_expires_at>?3 AND u.deleted_at IS NULL")
            .bind(&claims.sid).bind(&claims.sub).bind(now).fetch_one(&self.pool).await?;
        if count != 1 { return Err(Error::Unauthorized); }
        Ok(Actor { user_id: claims.sub, session_id: claims.sid })
    }

    pub async fn refresh(&self, token: &str) -> Result<Tokens> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) { return Err(Error::Unauthorized); }
        let now = self.clock.now();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT s.id,s.user_id,s.family_id,s.rotated_at,s.revoked_at,s.expires_at FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.refresh_hash=?1 AND u.deleted_at IS NULL")
            .bind(token_hash(token)).fetch_optional(&mut *transaction).await?.ok_or(Error::Unauthorized)?;
        let family: String = row.get("family_id");
        let user: String = row.get("user_id");
        if row.get::<Option<i64>, _>("rotated_at").is_some() {
            sqlx::query("UPDATE sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE family_id=?2").bind(now).bind(&family).execute(&mut *transaction).await?;
            audit::append(&mut transaction, Some(&user), "auth.refresh_reuse", "session_family", &family, now, true).await?;
            transaction.commit().await?;
            return Err(Error::Unauthorized);
        }
        if row.get::<Option<i64>, _>("revoked_at").is_some() || row.get::<i64, _>("expires_at") <= now { return Err(Error::Unauthorized); }
        let id: String = row.get("id");
        sqlx::query("UPDATE sessions SET rotated_at=?1 WHERE id=?2").bind(now).bind(&id).execute(&mut *transaction).await?;
        let tokens = self.issue_session(&mut transaction, &user, &family).await?;
        audit::append(&mut transaction, Some(&user), "auth.refresh", "session", &id, now, false).await?;
        transaction.commit().await?;
        Ok(tokens)
    }

    pub async fn logout(&self, actor: &Actor, all: bool) -> Result<()> {
        let now = self.clock.now();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if all {
            sqlx::query("UPDATE sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE user_id=?2")
                .bind(now).bind(&actor.user_id).execute(&mut *transaction).await?;
        } else {
            sqlx::query("UPDATE sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE family_id=(SELECT family_id FROM sessions WHERE id=?2 AND user_id=?3)")
                .bind(now).bind(&actor.session_id).bind(&actor.user_id).execute(&mut *transaction).await?;
        }
        audit::append(&mut transaction, Some(&actor.user_id), if all { "auth.revoke_all" } else { "auth.logout" }, "user", &actor.user_id, now, false).await?;
        transaction.commit().await?;
        Ok(())
    }
}