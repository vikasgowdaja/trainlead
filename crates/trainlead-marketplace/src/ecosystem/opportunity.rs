use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Actor, deny, record_event};
use crate::{Error, Result, audit, clock::Clock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum OpportunityStatus {
    Detected, Verified, Open, Sourcing, Staffed, InDelivery, Completed, Settled, Cancelled, Expired,
}

impl OpportunityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "Detected", Self::Verified => "Verified", Self::Open => "Open",
            Self::Sourcing => "Sourcing", Self::Staffed => "Staffed", Self::InDelivery => "InDelivery",
            Self::Completed => "Completed", Self::Settled => "Settled", Self::Cancelled => "Cancelled",
            Self::Expired => "Expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "Detected" => Self::Detected, "Verified" => Self::Verified, "Open" => Self::Open,
            "Sourcing" => Self::Sourcing, "Staffed" => Self::Staffed, "InDelivery" => Self::InDelivery,
            "Completed" => Self::Completed, "Settled" => Self::Settled, "Cancelled" => Self::Cancelled,
            "Expired" => Self::Expired,
            _ => return Err(Error::Validation("unknown opportunity status".into())),
        })
    }

    pub fn is_terminal(self) -> bool { matches!(self, Self::Settled | Self::Cancelled | Self::Expired) }

    pub fn can_transition(self, to: Self) -> bool {
        use OpportunityStatus::*;
        matches!((self, to),
            (Detected, Verified) | (Verified, Open) | (Open, Sourcing) | (Sourcing, Staffed)
            | (Staffed, Sourcing) | (Staffed, InDelivery) | (InDelivery, Completed) | (Completed, Settled)
            | (Detected | Verified | Open | Sourcing | Staffed | InDelivery, Cancelled)
            | (Detected | Verified | Open | Sourcing, Expired))
    }
}

pub async fn transition(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, id: &str, to: OpportunityStatus) -> Result<()> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "opportunity:transition", "opportunity", id, now).await); }
    let mut transaction = pool.begin().await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as("SELECT status, merged_into FROM opportunities WHERE id=?1")
        .bind(id).fetch_optional(&mut *transaction).await?;
    let (status, merged_into) = row.ok_or(Error::NotFound)?;
    if merged_into.is_some() { return Err(Error::Conflict("opportunity was merged; transition the target instead".into())); }
    let from = OpportunityStatus::parse(&status)?;
    if !from.can_transition(to) {
        return Err(Error::Conflict(format!("cannot move opportunity from {} to {}", from.as_str(), to.as_str())));
    }
    let changed = sqlx::query("UPDATE opportunities SET status=?1, updated_at=?2 WHERE id=?3 AND status=?4")
        .bind(to.as_str()).bind(now).bind(id).bind(from.as_str()).execute(&mut *transaction).await?.rows_affected();
    if changed != 1 { return Err(Error::Conflict("opportunity changed concurrently".into())); }
    record_event(&mut transaction, "opportunity", id, "status_changed", Some(from.as_str()), Some(to.as_str()),
        Some(actor.user_id()), &json!({}), now).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "opportunity:transition", "opportunity", id, now, false).await?;
    transaction.commit().await?;
    Ok(())
}

/// Moves every posting and non-duplicate chain link of `source` into `target`; recorded so it can be undone.
pub async fn merge(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, target_id: &str, source_id: &str) -> Result<String> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "opportunity:merge", "opportunity", source_id, now).await); }
    if target_id == source_id { return Err(Error::Validation("cannot merge an opportunity into itself".into())); }
    let mut transaction = pool.begin().await?;
    for id in [target_id, source_id] {
        let row: Option<(String, Option<String>)> = sqlx::query_as("SELECT status, merged_into FROM opportunities WHERE id=?1")
            .bind(id).fetch_optional(&mut *transaction).await?;
        let (status, merged_into) = row.ok_or(Error::NotFound)?;
        if merged_into.is_some() { return Err(Error::Conflict("opportunity is already merged".into())); }
        if OpportunityStatus::parse(&status)?.is_terminal() { return Err(Error::Conflict("cannot merge a closed opportunity".into())); }
    }
    let posting_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM postings WHERE opportunity_id=?1 ORDER BY id")
        .bind(source_id).fetch_all(&mut *transaction).await?;
    // Identical hops already on the target stay with the source and return with it on unmerge.
    let link_ids: Vec<String> = sqlx::query_scalar(
        "SELECT s.id FROM chain_links s WHERE s.opportunity_id=?1 AND NOT EXISTS (
            SELECT 1 FROM chain_links t WHERE t.opportunity_id=?2 AND t.from_kind=s.from_kind
            AND t.from_id=(CASE WHEN s.from_kind='unknown_client' THEN ?2 ELSE s.from_id END)
            AND t.to_kind=s.to_kind AND t.to_id=s.to_id) ORDER BY s.id")
        .bind(source_id).bind(target_id).fetch_all(&mut *transaction).await?;
    sqlx::query("UPDATE postings SET opportunity_id=?1 WHERE opportunity_id=?2")
        .bind(target_id).bind(source_id).execute(&mut *transaction).await?;
    for link in &link_ids {
        sqlx::query("UPDATE chain_links SET opportunity_id=?1,
                     from_id=CASE WHEN from_kind='unknown_client' THEN ?1 ELSE from_id END WHERE id=?2")
            .bind(target_id).bind(link).execute(&mut *transaction).await?;
    }
    sqlx::query("UPDATE opportunities SET merged_into=?1, updated_at=?2 WHERE id=?3")
        .bind(target_id).bind(now).bind(source_id).execute(&mut *transaction).await?;
    sqlx::query("UPDATE opportunities SET updated_at=?1 WHERE id=?2").bind(now).bind(target_id).execute(&mut *transaction).await?;
    let merge_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO opportunity_merges(id,target_id,source_id,posting_ids,link_ids,merged_by,merged_at) VALUES(?1,?2,?3,?4,?5,?6,?7)")
        .bind(&merge_id).bind(target_id).bind(source_id).bind(serde_json::to_string(&posting_ids)?)
        .bind(serde_json::to_string(&link_ids)?).bind(actor.user_id()).bind(now).execute(&mut *transaction).await?;
    let detail = json!({"merge_id": merge_id, "target": target_id, "source": source_id, "postings": posting_ids.len()});
    record_event(&mut transaction, "opportunity", target_id, "merged_in", None, None, Some(actor.user_id()), &detail, now).await?;
    record_event(&mut transaction, "opportunity", source_id, "merged_into", None, None, Some(actor.user_id()), &detail, now).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "opportunity:merge", "opportunity", source_id, now, false).await?;
    transaction.commit().await?;
    Ok(merge_id)
}

pub async fn unmerge(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, merge_id: &str) -> Result<()> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "opportunity:unmerge", "opportunity_merge", merge_id, now).await); }
    let mut transaction = pool.begin().await?;
    let row: Option<(String, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT target_id, source_id, posting_ids, link_ids, unmerged_at FROM opportunity_merges WHERE id=?1")
        .bind(merge_id).fetch_optional(&mut *transaction).await?;
    let (target_id, source_id, posting_ids, link_ids, unmerged_at) = row.ok_or(Error::NotFound)?;
    if unmerged_at.is_some() { return Err(Error::Conflict("merge was already undone".into())); }
    let merged_into: Option<Option<String>> = sqlx::query_scalar("SELECT merged_into FROM opportunities WHERE id=?1")
        .bind(&source_id).fetch_optional(&mut *transaction).await?;
    if merged_into.flatten().as_deref() != Some(target_id.as_str()) {
        return Err(Error::Conflict("source is no longer merged into the target".into()));
    }
    let posting_ids: Vec<String> = serde_json::from_str(&posting_ids)?;
    let link_ids: Vec<String> = serde_json::from_str(&link_ids)?;
    for posting in &posting_ids {
        let moved = sqlx::query("UPDATE postings SET opportunity_id=?1 WHERE id=?2 AND opportunity_id=?3")
            .bind(&source_id).bind(posting).bind(&target_id).execute(&mut *transaction).await?.rows_affected();
        if moved != 1 { return Err(Error::Conflict("a posting moved again after this merge; undo later merges first".into())); }
    }
    for link in &link_ids {
        let moved = sqlx::query("UPDATE chain_links SET opportunity_id=?1,
                                 from_id=CASE WHEN from_kind='unknown_client' THEN ?1 ELSE from_id END WHERE id=?2 AND opportunity_id=?3")
            .bind(&source_id).bind(link).bind(&target_id).execute(&mut *transaction).await?.rows_affected();
        if moved != 1 { return Err(Error::Conflict("a chain link moved again after this merge; undo later merges first".into())); }
    }
    sqlx::query("UPDATE opportunities SET merged_into=NULL, updated_at=?1 WHERE id=?2").bind(now).bind(&source_id).execute(&mut *transaction).await?;
    sqlx::query("UPDATE opportunity_merges SET unmerged_at=?1 WHERE id=?2").bind(now).bind(merge_id).execute(&mut *transaction).await?;
    let detail = json!({"merge_id": merge_id});
    record_event(&mut transaction, "opportunity", &target_id, "unmerged", None, None, Some(actor.user_id()), &detail, now).await?;
    record_event(&mut transaction, "opportunity", &source_id, "unmerged", None, None, Some(actor.user_id()), &detail, now).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "opportunity:unmerge", "opportunity", &source_id, now, false).await?;
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::OpportunityStatus::*;

    #[test]
    fn lifecycle_allows_only_documented_transitions() {
        let path = [Detected, Verified, Open, Sourcing, Staffed, InDelivery, Completed, Settled];
        for pair in path.windows(2) { assert!(pair[0].can_transition(pair[1]), "{:?} -> {:?}", pair[0], pair[1]); }
        assert!(!Detected.can_transition(Open));
        assert!(!Completed.can_transition(Cancelled));
        assert!(!InDelivery.can_transition(Expired));
        for terminal in [Settled, Cancelled, Expired] {
            assert!(terminal.is_terminal());
            assert!(path.iter().all(|to| !terminal.can_transition(*to)));
        }
    }
}
