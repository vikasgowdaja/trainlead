//! Ecosystem layer: one real-world need is one Opportunity with many Postings and a chain of custody.
pub mod chain;
pub mod clients;
pub mod ingest;
pub mod opportunity;
pub mod rates;
pub mod views;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};
use trainlead_core::models::Provision;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Admin,
    Vendor { org_id: String },
    Trainer { trainer_id: String },
}

/// An authenticated party. Constructors verify membership in the database, so it cannot be forged.
#[derive(Debug, Clone)]
pub struct Actor {
    user_id: String,
    role: Role,
}

impl Actor {
    pub fn user_id(&self) -> &str { &self.user_id }
    pub fn role(&self) -> &Role { &self.role }
    pub fn is_admin(&self) -> bool { self.role == Role::Admin }

    pub fn vendor_org(&self) -> Option<&str> {
        match &self.role { Role::Vendor { org_id } => Some(org_id), _ => None }
    }

    pub fn trainer_id(&self) -> Option<&str> {
        match &self.role { Role::Trainer { trainer_id } => Some(trainer_id), _ => None }
    }

    pub async fn admin(pool: &SqlitePool, user_id: &str) -> Result<Self> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM user_roles r JOIN users u ON u.id=r.user_id
             WHERE r.user_id=?1 AND r.role IN ('Admin','SuperAdmin') AND u.deleted_at IS NULL LIMIT 1")
            .bind(user_id).fetch_optional(pool).await?;
        found.map(|_| Self { user_id: user_id.into(), role: Role::Admin }).ok_or(Error::Forbidden)
    }

    pub async fn vendor(pool: &SqlitePool, user_id: &str, org_id: &str) -> Result<Self> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM org_members m JOIN organizations o ON o.id=m.org_id JOIN users u ON u.id=m.user_id
             WHERE m.user_id=?1 AND m.org_id=?2 AND o.deleted_at IS NULL AND u.deleted_at IS NULL")
            .bind(user_id).bind(org_id).fetch_optional(pool).await?;
        found.map(|_| Self { user_id: user_id.into(), role: Role::Vendor { org_id: org_id.into() } })
            .ok_or(Error::Forbidden)
    }

    pub async fn trainer(pool: &SqlitePool, user_id: &str) -> Result<Self> {
        let trainer_id: Option<String> = sqlx::query_scalar(
            "SELECT t.id FROM trainers t JOIN users u ON u.id=t.user_id
             WHERE t.user_id=?1 AND t.deleted_at IS NULL AND u.deleted_at IS NULL")
            .bind(user_id).fetch_optional(pool).await?;
        trainer_id.map(|trainer_id| Self { user_id: user_id.into(), role: Role::Trainer { trainer_id } })
            .ok_or(Error::Forbidden)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartyKind { Client, UnknownClient, Vendor, External, Trainer }

impl PartyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::UnknownClient => "unknown_client",
            Self::Vendor => "vendor",
            Self::External => "external",
            Self::Trainer => "trainer",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "client" => Self::Client,
            "unknown_client" => Self::UnknownClient,
            "vendor" => Self::Vendor,
            "external" => Self::External,
            "trainer" => Self::Trainer,
            _ => return Err(Error::Validation("unknown party kind".into())),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogisticsTerm {
    Provided,
    Reimbursed { cap_inr: Option<i64> },
    OwnCost,
    Unknown,
}

impl From<Provision> for LogisticsTerm {
    fn from(value: Provision) -> Self {
        match value {
            Provision::Provided => Self::Provided,
            Provision::NotProvided => Self::OwnCost,
            Provision::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct Logistics {
    pub travel: LogisticsTerm,
    pub food: LogisticsTerm,
    pub accommodation: LogisticsTerm,
    pub note: Option<String>,
}

impl Default for Logistics {
    fn default() -> Self {
        Self { travel: LogisticsTerm::Unknown, food: LogisticsTerm::Unknown, accommodation: LogisticsTerm::Unknown, note: None }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_event(
    transaction: &mut Transaction<'_, Sqlite>, object_type: &str, object_id: &str, event: &str,
    from_state: Option<&str>, to_state: Option<&str>, actor: Option<&str>, detail: &Value, now: i64,
) -> Result<()> {
    sqlx::query("INSERT INTO object_events(id,object_type,object_id,event,from_state,to_state,actor_id,detail,occurred_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
        .bind(Uuid::new_v4().to_string()).bind(object_type).bind(object_id).bind(event)
        .bind(from_state).bind(to_state).bind(actor).bind(detail.to_string()).bind(now)
        .execute(&mut **transaction).await?;
    Ok(())
}

pub(crate) async fn deny(pool: &SqlitePool, actor: &Actor, action: &str, object_type: &str, object_id: &str, now: i64) -> Error {
    let result = async {
        let mut transaction = pool.begin().await?;
        crate::audit::append(&mut transaction, Some(actor.user_id()), action, object_type, object_id, now, true).await?;
        transaction.commit().await?;
        Ok::<_, Error>(())
    }.await;
    match result { Ok(()) => Error::Forbidden, Err(error) => error }
}

#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct ObjectEvent {
    pub event: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub actor_id: Option<String>,
    pub occurred_at: i64,
}

pub async fn events(pool: &SqlitePool, object_type: &str, object_id: &str) -> Result<Vec<ObjectEvent>> {
    Ok(sqlx::query_as(
        "SELECT event,from_state,to_state,actor_id,occurred_at FROM object_events
         WHERE object_type=?1 AND object_id=?2 ORDER BY occurred_at, rowid")
        .bind(object_type).bind(object_id).fetch_all(pool).await?)
}
