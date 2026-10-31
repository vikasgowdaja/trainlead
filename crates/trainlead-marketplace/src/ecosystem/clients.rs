use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use trainlead_core::dedup::{normalized_text, token_set_ratio};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Actor, deny, record_event};
use crate::{Error, Result, audit, clock::Clock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind { College, University, Corporate, Institute, Other }

impl ClientKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::College => "college", Self::University => "university", Self::Corporate => "corporate",
            Self::Institute => "institute", Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClientContact {
    pub name: String,
    pub role: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewClient {
    pub name: String,
    pub kind: ClientKind,
    pub city: Option<String>,
    pub state: Option<String>,
    pub contacts: Vec<ClientContact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum ClientMatch {
    Confirmed { client_id: String },
    Suggested { client_id: String, name: String, score: f64 },
    None,
}

const CONFIRM_SORT_RATIO: f64 = 95.0;
const SUGGEST_SET_RATIO: f64 = 80.0;
const CITY_RATIO: f64 = 80.0;

/// Normalizes spelling variants so "Mysore Engg College" and "Mysuru Engineering College" compare equal.
pub fn canonical_name(name: &str) -> String {
    normalized_text(name).split_whitespace().map(|word| match word {
        "engg" | "eng" => "engineering", "inst" => "institute", "tech" => "technology",
        "univ" => "university", "clg" | "coll" => "college", "mysore" => "mysuru",
        "bangalore" => "bengaluru", "bombay" => "mumbai", "madras" => "chennai", "calcutta" => "kolkata",
        "poona" => "pune", "mangalore" => "mangaluru", "belgaum" => "belagavi", "hubli" => "hubballi",
        "gurgaon" => "gurugram", "trivandrum" => "thiruvananthapuram",
        other => other,
    }).collect::<Vec<_>>().join(" ")
}

fn token_sort_ratio(left: &str, right: &str) -> f64 {
    let sorted = |text: &str| { let mut words: Vec<&str> = text.split_whitespace().collect(); words.sort_unstable(); words.join(" ") };
    strsim::normalized_levenshtein(&sorted(left), &sorted(right)) * 100.0
}

pub async fn create_client(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, client: NewClient) -> Result<String> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "client:create", "client", "-", now).await); }
    let name = client.name.trim();
    if name.is_empty() || name.len() > 200 { return Err(Error::Validation("client name must be 1-200 characters".into())); }
    let id = Uuid::new_v4().to_string();
    let mut transaction = pool.begin().await?;
    sqlx::query("INSERT INTO clients(id,name,kind,city,state,contacts,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)")
        .bind(&id).bind(name).bind(client.kind.as_str()).bind(client.city.as_deref().map(str::trim))
        .bind(client.state.as_deref().map(str::trim)).bind(serde_json::to_string(&client.contacts)?).bind(now)
        .execute(&mut *transaction).await?;
    insert_alias(&mut transaction, &id, name).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "client:create", "client", &id, now, false).await?;
    transaction.commit().await?;
    Ok(id)
}

async fn insert_alias(transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>, client_id: &str, alias: &str) -> Result<()> {
    let normalized = canonical_name(alias);
    if normalized.is_empty() { return Ok(()); }
    let existing: Option<String> = sqlx::query_scalar("SELECT client_id FROM client_aliases WHERE normalized=?1")
        .bind(&normalized).fetch_optional(&mut **transaction).await?;
    match existing {
        Some(owner) if owner != client_id => Err(Error::Conflict("alias already belongs to another client".into())),
        Some(_) => Ok(()),
        None => {
            sqlx::query("INSERT INTO client_aliases(normalized,client_id) VALUES(?1,?2)")
                .bind(&normalized).bind(client_id).execute(&mut **transaction).await?;
            Ok(())
        }
    }
}

pub async fn resolve_client(pool: &SqlitePool, name: &str, city: Option<&str>) -> Result<ClientMatch> {
    let canonical = canonical_name(name);
    if canonical.is_empty() { return Ok(ClientMatch::None); }
    let exact: Option<String> = sqlx::query_scalar(
        "SELECT a.client_id FROM client_aliases a JOIN clients c ON c.id=a.client_id WHERE a.normalized=?1 AND c.deleted_at IS NULL")
        .bind(&canonical).fetch_optional(pool).await?;
    if let Some(client_id) = exact { return Ok(ClientMatch::Confirmed { client_id }); }
    let candidates: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.name, a.normalized, c.city FROM client_aliases a JOIN clients c ON c.id=a.client_id WHERE c.deleted_at IS NULL")
        .fetch_all(pool).await?;
    let city = city.map(canonical_name);
    let mut best: Option<(f64, f64, String, String)> = None;
    for (client_id, client_name, alias, client_city) in candidates {
        if let (Some(city), Some(client_city)) = (&city, client_city.as_deref().map(canonical_name))
            && token_set_ratio(city, &client_city) < CITY_RATIO { continue; }
        let sort = token_sort_ratio(&canonical, &alias);
        let set = token_set_ratio(&canonical, &alias);
        if best.as_ref().is_none_or(|(best_sort, best_set, _, _)| (sort, set) > (*best_sort, *best_set)) {
            best = Some((sort, set, client_id, client_name));
        }
    }
    Ok(match best {
        Some((sort, _, client_id, _)) if sort >= CONFIRM_SORT_RATIO => ClientMatch::Confirmed { client_id },
        Some((_, set, client_id, name)) if set >= SUGGEST_SET_RATIO => ClientMatch::Suggested { client_id, name, score: set },
        _ => ClientMatch::None,
    })
}

/// Admin confirms that an opportunity belongs to a client; the opportunity's college text becomes an alias.
pub async fn confirm_client(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, opportunity_id: &str, client_id: &str) -> Result<()> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "client:confirm", "opportunity", opportunity_id, now).await); }
    let mut transaction = pool.begin().await?;
    let college: Option<Option<String>> = sqlx::query_scalar("SELECT college FROM opportunities WHERE id=?1")
        .bind(opportunity_id).fetch_optional(&mut *transaction).await?;
    let college = college.ok_or(Error::NotFound)?;
    let client: Option<i64> = sqlx::query_scalar("SELECT 1 FROM clients WHERE id=?1 AND deleted_at IS NULL")
        .bind(client_id).fetch_optional(&mut *transaction).await?;
    client.ok_or(Error::NotFound)?;
    sqlx::query("UPDATE opportunities SET client_id=?1, client_match='confirmed', suggested_client_id=NULL, updated_at=?2 WHERE id=?3")
        .bind(client_id).bind(now).bind(opportunity_id).execute(&mut *transaction).await?;
    sqlx::query("UPDATE chain_links SET from_kind='client', from_id=?1 WHERE opportunity_id=?2 AND from_kind='unknown_client'")
        .bind(client_id).bind(opportunity_id).execute(&mut *transaction).await?;
    if let Some(college) = college { insert_alias(&mut transaction, client_id, &college).await?; }
    record_event(&mut transaction, "opportunity", opportunity_id, "client_confirmed", None, None,
        Some(actor.user_id()), &json!({"client_id": client_id}), now).await?;
    audit::append(&mut transaction, Some(actor.user_id()), "client:confirm", "opportunity", opportunity_id, now, false).await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn reject_client_suggestion(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, opportunity_id: &str) -> Result<()> {
    let now = clock.now();
    if !actor.is_admin() { return Err(deny(pool, actor, "client:reject", "opportunity", opportunity_id, now).await); }
    let mut transaction = pool.begin().await?;
    let changed = sqlx::query("UPDATE opportunities SET client_match='none', suggested_client_id=NULL, updated_at=?1 WHERE id=?2 AND client_match='suggested'")
        .bind(now).bind(opportunity_id).execute(&mut *transaction).await?.rows_affected();
    if changed != 1 { return Err(Error::Conflict("opportunity has no pending client suggestion".into())); }
    record_event(&mut transaction, "opportunity", opportunity_id, "client_suggestion_rejected", None, None,
        Some(actor.user_id()), &json!({}), now).await?;
    transaction.commit().await?;
    Ok(())
}
