use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Result;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct AuditEntry {
    pub id: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub object_type: String,
    pub object_id: String,
    pub occurred_at: i64,
    pub outcome: String,
}

pub(crate) async fn append(
    transaction: &mut Transaction<'_, Sqlite>, actor: Option<&str>, action: &str,
    object_type: &str, object_id: &str, now: i64, denied: bool,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_log(id,actor_id,action,object_type,object_id,occurred_at,outcome) VALUES(?1,?2,?3,?4,?5,?6,?7)")
        .bind(Uuid::new_v4().to_string()).bind(actor).bind(action).bind(object_type).bind(object_id).bind(now)
        .bind(if denied { "denied" } else { "success" }).execute(&mut **transaction).await?;
    Ok(())
}