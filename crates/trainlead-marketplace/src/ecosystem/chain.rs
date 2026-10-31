use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Actor, PartyKind, deny, opportunity::OpportunityStatus, rates::Rate, record_event};
use crate::{Error, Result, audit, clock::Clock};

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Recipient {
    Vendor(String),
    Trainer(String),
}

/// A vendor involved in an opportunity hands it on. `disclose` reveals the rate to the recipient (an offer).
pub async fn add_link(
    pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, opportunity_id: &str, to: &Recipient,
    rate: Option<Rate>, disclose: bool,
) -> Result<String> {
    let now = clock.now();
    let Some(org_id) = actor.vendor_org() else {
        return Err(deny(pool, actor, "chain:link", "opportunity", opportunity_id, now).await);
    };
    if rate.is_some_and(|rate| rate.amount_inr <= 0) { return Err(Error::Validation("rate must be positive".into())); }
    let involved: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM postings WHERE opportunity_id=?1 AND posted_by_org_id=?2
         UNION SELECT 1 FROM chain_links WHERE opportunity_id=?1 AND to_kind='vendor' AND to_id=?2 LIMIT 1")
        .bind(opportunity_id).bind(org_id).fetch_optional(pool).await?;
    if involved.is_none() { return Err(deny(pool, actor, "chain:link", "opportunity", opportunity_id, now).await); }
    let mut transaction = pool.begin().await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as("SELECT status, merged_into FROM opportunities WHERE id=?1")
        .bind(opportunity_id).fetch_optional(&mut *transaction).await?;
    let (status, merged_into) = row.ok_or(Error::NotFound)?;
    if merged_into.is_some() || OpportunityStatus::parse(&status)?.is_terminal() {
        return Err(Error::Conflict("opportunity is not accepting new links".into()));
    }
    let (to_kind, to_id) = match to {
        Recipient::Vendor(id) => {
            if id == org_id { return Err(Error::Validation("cannot link a vendor to itself".into())); }
            let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM organizations WHERE id=?1 AND kind='vendor' AND deleted_at IS NULL")
                .bind(id).fetch_optional(&mut *transaction).await?;
            exists.ok_or(Error::NotFound)?;
            (PartyKind::Vendor, id)
        }
        Recipient::Trainer(id) => {
            let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM trainers WHERE id=?1 AND deleted_at IS NULL")
                .bind(id).fetch_optional(&mut *transaction).await?;
            exists.ok_or(Error::NotFound)?;
            (PartyKind::Trainer, id)
        }
    };
    let id = Uuid::new_v4().to_string();
    let inserted = sqlx::query("INSERT INTO chain_links(id,opportunity_id,from_kind,from_id,to_kind,to_id,rate_amount_inr,rate_unit,
                                disclosed_to_recipient,inferred,created_at) VALUES(?1,?2,'vendor',?3,?4,?5,?6,?7,?8,0,?9)
                                ON CONFLICT(opportunity_id,from_kind,from_id,to_kind,to_id) DO NOTHING")
        .bind(&id).bind(opportunity_id).bind(org_id).bind(to_kind.as_str()).bind(to_id)
        .bind(rate.map(|rate| rate.amount_inr)).bind(rate.and_then(|rate| rate.unit).map(|unit| unit.as_str()))
        .bind(disclose).bind(now).execute(&mut *transaction).await?.rows_affected();
    if inserted != 1 { return Err(Error::Conflict("this hop already exists".into())); }
    record_event(&mut transaction, "opportunity", opportunity_id, "chain_link_added", None, None, Some(actor.user_id()),
        &json!({"link_id": id, "to_kind": to_kind.as_str(), "disclosed": disclose}), now).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "chain:link", "chain_link", &id, now, false).await?;
    transaction.commit().await?;
    Ok(id)
}
