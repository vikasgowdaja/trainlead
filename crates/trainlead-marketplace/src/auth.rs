use std::sync::Arc;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{SqlitePool, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{Error, Result, audit, clock::Clock, password};

#[derive(Clone)]
pub struct AuthService {
    pub(crate) pool: SqlitePool,
    pub(crate) secret: Arc<Vec<u8>>,
    pub(crate) clock: Arc<dyn Clock>,
    dummy_hash: String,
}

#[derive(Debug, Clone)]
pub struct Actor { pub(crate) user_id: String, pub(crate) session_id: String }

impl Actor {
    pub fn user_id(&self) -> &str { &self.user_id }
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UserSummary {
    pub id: String,
    pub phone_verified: bool,
    pub email_verified: bool,
    pub created_at: i64,
}

impl AuthService {
    pub async fn new(pool: SqlitePool, secret: Vec<u8>, clock: Arc<dyn Clock>) -> Result<Self> {
        if secret.len() < 32 { return Err(Error::Configuration); }
        let dummy_hash = password::hash(crate::sessions::random_token()).await?;
        Ok(Self { pool, secret: Arc::new(secret), clock, dummy_hash })
    }

    pub(crate) fn keyed_hash(&self, purpose: &str, value: &str) -> Result<String> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).map_err(|_| Error::Crypto)?;
        mac.update(purpose.as_bytes()); mac.update(&[0]); mac.update(value.as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    pub async fn register(&self, email: &str, password: String, consent: bool) -> Result<()> {
        if !consent { return Err(Error::Validation("personal-data consent is required".into())); }
        let email = normalize_email(email)?;
        password::validate(&password)?;
        let hash = password::hash(password).await?;
        let id = Uuid::new_v4().to_string();
        let now = self.clock.now();
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT INTO users(id,email,password_hash,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(email) DO NOTHING")
            .bind(&id).bind(email).bind(hash).bind(now).execute(&mut *transaction).await?.rows_affected();
        if inserted == 1 {
            sqlx::query("INSERT INTO user_roles(user_id,role) VALUES(?1,'Viewer')").bind(&id).execute(&mut *transaction).await?;
            audit::append(&mut transaction, Some(&id), "user.register", "user", &id, now, false).await?;
            audit::append(&mut transaction, Some(&id), "consent.personal_data", "user", &id, now, false).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn bootstrap(&self, email: &str, password: String) -> Result<()> {
        let email = normalize_email(email)?;
        password::validate(&password)?;
        let hash = password::hash(password).await?;
        let id = Uuid::new_v4().to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&mut *transaction).await?;
        if count != 0 { return Err(Error::Forbidden); }
        sqlx::query("INSERT INTO users(id,email,password_hash,created_at) VALUES(?1,?2,?3,?4)")
            .bind(&id).bind(email).bind(hash).bind(self.clock.now()).execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role) VALUES(?1,'SuperAdmin')").bind(&id).execute(&mut *transaction).await?;
        audit::append(&mut transaction, Some(&id), "user.bootstrap", "user", &id, self.clock.now(), false).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn login(&self, email: &str, password: String) -> Result<Tokens> {
        let email = normalize_email(email).map_err(|_| Error::Unauthorized)?;
        if password.len() > 128 { return Err(Error::Unauthorized); }
        let bucket = self.keyed_hash("login_failure", &email)?;
        let locked_until: Option<i64> = sqlx::query_scalar("SELECT locked_until FROM auth_limits WHERE bucket=?1")
            .bind(&bucket).fetch_optional(&self.pool).await?;
        let user = sqlx::query("SELECT id,password_hash FROM users WHERE email=?1 AND deleted_at IS NULL")
            .bind(&email).fetch_optional(&self.pool).await?;
        let hash = user.as_ref().and_then(|row| row.get::<Option<String>, _>("password_hash"))
            .unwrap_or_else(|| self.dummy_hash.clone());
        let verified = password::verify(password, hash).await?;
        let locked = locked_until.is_some_and(|until| until > self.clock.now());
        if !verified || user.is_none() || locked {
            self.login_failure(&bucket).await?;
            return Err(Error::Unauthorized);
        }
        let user = user.ok_or(Error::Unauthorized)?;
        let id: String = user.get("id");
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM auth_limits WHERE bucket=?1").bind(bucket).execute(&mut *transaction).await?;
        let tokens = self.issue_session(&mut transaction, &id, &Uuid::new_v4().to_string()).await?;
        audit::append(&mut transaction, Some(&id), "auth.login", "user", &id, self.clock.now(), false).await?;
        transaction.commit().await?;
        Ok(tokens)
    }

    async fn login_failure(&self, bucket: &str) -> Result<()> {
        let now = self.clock.now();
        let mut transaction = self.pool.begin().await?;
        let count: i64 = sqlx::query_scalar("INSERT INTO auth_limits(bucket,window_started,count) VALUES(?1,?2,1)
            ON CONFLICT(bucket) DO UPDATE SET count=CASE WHEN auth_limits.window_started<=?2-900 THEN 1 ELSE auth_limits.count+1 END,
            window_started=CASE WHEN auth_limits.window_started<=?2-900 THEN ?2 ELSE auth_limits.window_started END RETURNING count")
            .bind(bucket).bind(now).fetch_one(&mut *transaction).await?;
        if count >= 5 {
            sqlx::query("UPDATE auth_limits SET locked_until=?1 WHERE bucket=?2").bind(now + 900).bind(bucket).execute(&mut *transaction).await?;
        }
        audit::append(&mut transaction, None, "auth.login", "authentication", "redacted", now, true).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn rate_limit(&self, purpose: &str, identifier: &str, limit: i64, seconds: i64) -> Result<()> {
        let bucket = self.keyed_hash(purpose, identifier)?;
        let now = self.clock.now();
        let count: i64 = sqlx::query_scalar("INSERT INTO auth_limits(bucket,window_started,count) VALUES(?1,?2,1)
            ON CONFLICT(bucket) DO UPDATE SET count=CASE WHEN auth_limits.window_started<=?2-?3 THEN 1 ELSE auth_limits.count+1 END,
            window_started=CASE WHEN auth_limits.window_started<=?2-?3 THEN ?2 ELSE auth_limits.window_started END RETURNING count")
            .bind(bucket).bind(now).bind(seconds).fetch_one(&self.pool).await?;
        if count > limit { return Err(Error::RateLimited); }
        Ok(())
    }

    pub async fn require(&self, actor: &Actor, permission: &str) -> Result<()> {
        let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_roles ur JOIN roles r ON r.name=ur.role
            JOIN role_permissions rp ON rp.role=ur.role JOIN users u ON u.id=ur.user_id
            WHERE ur.user_id=?1 AND rp.permission=?2 AND r.scope='global' AND u.deleted_at IS NULL")
            .bind(&actor.user_id).bind(permission).fetch_one(&self.pool).await?;
        if allowed > 0 { return Ok(()); }
        let mut transaction = self.pool.begin().await?;
        audit::append(&mut transaction, Some(&actor.user_id), "authorization.denied", "permission", permission, self.clock.now(), true).await?;
        transaction.commit().await?;
        Err(Error::Forbidden)
    }

    pub async fn assign_role(&self, actor: &Actor, user_id: &str, role: &str) -> Result<()> {
        self.require(actor, "role:manage").await?;
        let mut transaction = self.pool.begin().await?;
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE name=?1 AND scope='global'")
            .bind(role).fetch_one(&mut *transaction).await?;
        let user_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id=?1 AND deleted_at IS NULL")
            .bind(user_id).fetch_one(&mut *transaction).await?;
        if exists != 1 || user_exists != 1 { return Err(Error::Validation("invalid global role or user".into())); }
        sqlx::query("INSERT INTO user_roles(user_id,role) VALUES(?1,?2) ON CONFLICT DO NOTHING")
            .bind(user_id).bind(role).execute(&mut *transaction).await?;
        audit::append(&mut transaction, Some(&actor.user_id), "role.assign", "user", user_id, self.clock.now(), false).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn users(&self, actor: &Actor, offset: u32, limit: u32) -> Result<Vec<UserSummary>> {
        self.require(actor, "user:read").await?;
        let rows = sqlx::query("SELECT id,phone_verified,email_verified,created_at FROM users WHERE deleted_at IS NULL ORDER BY created_at,id LIMIT ?1 OFFSET ?2")
            .bind(limit.clamp(1,100)).bind(offset).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|row| UserSummary { id: row.get("id"), phone_verified: row.get("phone_verified"),
            email_verified: row.get("email_verified"), created_at: row.get("created_at") }).collect())
    }

    pub async fn audit_entries(&self, actor: &Actor, offset: u32, limit: u32) -> Result<Vec<audit::AuditEntry>> {
        self.require(actor, "audit:read").await?;
        Ok(sqlx::query_as("SELECT id,actor_id,action,object_type,object_id,occurred_at,outcome FROM audit_log ORDER BY occurred_at DESC,id DESC LIMIT ?1 OFFSET ?2")
            .bind(limit.clamp(1,100)).bind(offset).fetch_all(&self.pool).await?)
    }
}

pub(crate) fn normalize_email(value: &str) -> Result<String> {
    let email = value.trim().to_lowercase();
    let valid = email.len() <= 254 && !email.chars().any(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.') && !domain.starts_with('.'));
    if !valid || email.matches('@').count() != 1 { return Err(Error::Validation("invalid email format".into())); }
    Ok(email)
}