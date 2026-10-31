use chrono::{DateTime, FixedOffset, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::{Sqlite, SqlitePool, Transaction};
use trainlead_core::models::{Cluster, Sighting};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Logistics, PartyKind, clients::{ClientMatch, resolve_client}, rates::parse_rate, record_event};
use crate::{Error, Result, clock::Clock};

#[derive(Debug, Default, Clone, Serialize, ToSchema)]
pub struct ImportReport {
    pub opportunities_created: usize,
    pub postings_created: usize,
    pub postings_skipped: usize,
    pub clients_confirmed: usize,
    pub clients_suggested: usize,
}

const COMPLETENESS_FIELDS: f64 = 20.0;

/// Export timestamps carry no offset; WhatsApp exports from Indian phones are assumed to be IST.
pub fn to_utc(timestamp: &str) -> Result<String> {
    let parsed = match DateTime::parse_from_rfc3339(timestamp) {
        Ok(value) => value.with_timezone(&Utc),
        Err(_) => {
            let naive = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S")
                .map_err(|_| Error::Validation("posting timestamp is not ISO 8601".into()))?;
            let ist = FixedOffset::east_opt(19_800).ok_or(Error::Configuration)?;
            ist.from_local_datetime(&naive).single().ok_or(Error::Validation("ambiguous local timestamp".into()))?
                .with_timezone(&Utc)
        }
    };
    Ok(parsed.to_rfc3339_opts(SecondsFormat::Secs, true))
}

/// Idempotently imports dedup clusters: each cluster becomes an Opportunity, each sighting a Posting.
pub async fn import_clusters(pool: &SqlitePool, clock: &dyn Clock, clusters: &[Cluster]) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    for cluster in clusters {
        let primary = &cluster.primary.requirement;
        let resolution = match &primary.college {
            Some(college) => resolve_client(pool, college, primary.city.as_deref()).await?,
            None => ClientMatch::None,
        };
        let now = clock.now();
        let mut transaction = pool.begin().await?;
        let opportunity_id = match existing_opportunity(&mut transaction, cluster.id).await? {
            Some(id) => id,
            None => {
                let id = create_opportunity(&mut transaction, cluster, &resolution, now).await?;
                report.opportunities_created += 1;
                match resolution {
                    ClientMatch::Confirmed { .. } => report.clients_confirmed += 1,
                    ClientMatch::Suggested { .. } => report.clients_suggested += 1,
                    ClientMatch::None => {}
                }
                id
            }
        };
        let mut sources: Vec<&Sighting> = cluster.sources.iter().collect();
        sources.sort_by_key(|source| (to_utc(&source.message.timestamp).unwrap_or_default(), source.id));
        for sighting in sources {
            let imported: Option<i64> = sqlx::query_scalar("SELECT 1 FROM postings WHERE source_sighting_id=?1")
                .bind(sighting.id).fetch_optional(&mut *transaction).await?;
            if imported.is_some() { report.postings_skipped += 1; continue; }
            insert_posting(&mut transaction, &opportunity_id, sighting, now).await?;
            report.postings_created += 1;
        }
        infer_chain(&mut transaction, &opportunity_id, now).await?;
        transaction.commit().await?;
    }
    Ok(report)
}

/// Follows merges so new sightings of an already merged cluster land on the surviving opportunity.
async fn existing_opportunity(transaction: &mut Transaction<'_, Sqlite>, cluster_id: i64) -> Result<Option<String>> {
    let mut current: Option<String> = sqlx::query_scalar("SELECT id FROM opportunities WHERE source_cluster_id=?1")
        .bind(cluster_id).fetch_optional(&mut **transaction).await?;
    for _ in 0..32 {
        let Some(id) = current.clone() else { return Ok(None) };
        let next: Option<String> = sqlx::query_scalar("SELECT merged_into FROM opportunities WHERE id=?1")
            .bind(&id).fetch_one(&mut **transaction).await?;
        match next { Some(next) => current = Some(next), None => return Ok(Some(id)) }
    }
    Err(Error::Conflict("merge chain is too deep".into()))
}

async fn create_opportunity(transaction: &mut Transaction<'_, Sqlite>, cluster: &Cluster, resolution: &ClientMatch, now: i64) -> Result<String> {
    let requirement = &cluster.primary.requirement;
    let id = Uuid::new_v4().to_string();
    let (client_id, client_match, suggested) = match resolution {
        ClientMatch::Confirmed { client_id } => (Some(client_id.as_str()), "confirmed", None),
        ClientMatch::Suggested { client_id, .. } => (None, "suggested", Some(client_id.as_str())),
        ClientMatch::None => (None, "none", None),
    };
    let logistics = Logistics {
        travel: requirement.travel.into(), food: requirement.food.into(),
        accommodation: requirement.accommodation.into(), note: requirement.logistics_note.clone(),
    };
    let mode = requirement.mode.map(serde_json::to_value).transpose()?
        .and_then(|value| value.as_str().map(str::to_string));
    let confidence = (requirement.completeness() as f64 / COMPLETENESS_FIELDS).clamp(0.0, 1.0);
    sqlx::query("INSERT INTO opportunities(id,status,tech_stack,start_date,end_date,college,city,state,mode,audience,summary,
                 logistics,client_id,client_match,suggested_client_id,confidence,source_cluster_id,created_at,updated_at)
                 VALUES(?1,'Detected',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?17)")
        .bind(&id).bind(serde_json::to_string(&requirement.tech_stack)?)
        .bind(requirement.start_date.map(|date| date.to_string())).bind(requirement.end_date.map(|date| date.to_string()))
        .bind(&requirement.college).bind(&requirement.city).bind(&requirement.state).bind(mode)
        .bind(&requirement.audience).bind(&requirement.summary).bind(serde_json::to_string(&logistics)?)
        .bind(client_id).bind(client_match).bind(suggested).bind(confidence).bind(cluster.id).bind(now)
        .execute(&mut **transaction).await?;
    record_event(transaction, "opportunity", &id, "detected", None, Some("Detected"), None,
        &json!({"source_cluster_id": cluster.id, "client_match": client_match}), now).await?;
    Ok(id)
}

async fn insert_posting(transaction: &mut Transaction<'_, Sqlite>, opportunity_id: &str, sighting: &Sighting, now: i64) -> Result<()> {
    let requirement = &sighting.requirement;
    let (org_id, external_id) = resolve_poster(transaction, sighting, now).await?;
    let rate = requirement.rate.as_deref().and_then(parse_rate);
    let logistics = Logistics {
        travel: requirement.travel.into(), food: requirement.food.into(),
        accommodation: requirement.accommodation.into(), note: requirement.logistics_note.clone(),
    };
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO postings(id,opportunity_id,posted_by_org_id,external_contact_id,channel,rate_text,rate_amount_inr,
                 rate_unit,payment_terms,payment_date,logistics,contact_name,contact_phone,raw_text,source_group,sender,posted_at,
                 source_sighting_id,created_at) VALUES(?1,?2,?3,?4,'whatsapp',?5,?6,?7,NULL,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)")
        .bind(&id).bind(opportunity_id).bind(&org_id).bind(&external_id).bind(&requirement.rate)
        .bind(rate.map(|rate| rate.amount_inr)).bind(rate.and_then(|rate| rate.unit).map(|unit| unit.as_str()))
        .bind(requirement.payment_date.map(|date| date.to_string())).bind(serde_json::to_string(&logistics)?)
        .bind(&requirement.vendor_name).bind(&requirement.vendor_phone).bind(&sighting.message.raw_text)
        .bind(&sighting.message.source_group).bind(&sighting.message.sender).bind(to_utc(&sighting.message.timestamp)?)
        .bind(sighting.id).bind(now).execute(&mut **transaction).await?;
    record_event(transaction, "opportunity", opportunity_id, "posting_added", None, None, None,
        &json!({"posting_id": id, "source_sighting_id": sighting.id}), now).await
}

/// Known vendor phone numbers map to organizations; everyone else becomes an external contact.
async fn resolve_poster(transaction: &mut Transaction<'_, Sqlite>, sighting: &Sighting, now: i64) -> Result<(Option<String>, Option<String>)> {
    let phone = sighting.requirement.vendor_phone.as_deref();
    if let Some(phone) = phone {
        let org: Option<String> = sqlx::query_scalar(
            "SELECT v.org_id FROM vendor_contacts v JOIN organizations o ON o.id=v.org_id WHERE v.phone=?1 AND o.deleted_at IS NULL")
            .bind(phone).fetch_optional(&mut **transaction).await?;
        if org.is_some() { return Ok((org, None)); }
    }
    let name = sighting.requirement.vendor_name.clone().or_else(|| sighting.message.sender.clone());
    let existing: Option<String> = match phone {
        Some(phone) => sqlx::query_scalar("SELECT id FROM external_contacts WHERE phone=?1").bind(phone)
            .fetch_optional(&mut **transaction).await?,
        None => sqlx::query_scalar("SELECT id FROM external_contacts WHERE phone IS NULL AND name IS ?1 AND name IS NOT NULL")
            .bind(&name).fetch_optional(&mut **transaction).await?,
    };
    if let Some(id) = existing { return Ok((None, Some(id))); }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO external_contacts(id,name,phone,first_seen_at) VALUES(?1,?2,?3,?4)")
        .bind(&id).bind(name).bind(phone).bind(now).execute(&mut **transaction).await?;
    Ok((None, Some(id)))
}

#[derive(sqlx::FromRow)]
struct PosterRow {
    posted_by_org_id: Option<String>,
    external_contact_id: Option<String>,
    rate_amount_inr: Option<i64>,
    rate_unit: Option<String>,
}

/// Infers client -> first poster -> next poster ... in posting order; each hop carries the upstream poster's rate.
async fn infer_chain(transaction: &mut Transaction<'_, Sqlite>, opportunity_id: &str, now: i64) -> Result<()> {
    let client: Option<Option<String>> = sqlx::query_scalar("SELECT client_id FROM opportunities WHERE id=?1")
        .bind(opportunity_id).fetch_optional(&mut **transaction).await?;
    let client = client.ok_or(Error::NotFound)?;
    let rows: Vec<PosterRow> = sqlx::query_as(
        "SELECT posted_by_org_id, external_contact_id, rate_amount_inr, rate_unit FROM postings
         WHERE opportunity_id=?1 ORDER BY posted_at, created_at, id")
        .bind(opportunity_id).fetch_all(&mut **transaction).await?;
    let mut posters: Vec<(PartyKind, String, Option<i64>, Option<String>)> = Vec::new();
    for PosterRow { posted_by_org_id: org, external_contact_id: external, rate_amount_inr: amount, rate_unit: unit } in rows {
        let party = match (org, external) {
            (Some(org), _) => (PartyKind::Vendor, org),
            (None, Some(external)) => (PartyKind::External, external),
            (None, None) => continue,
        };
        if !posters.iter().any(|(kind, id, _, _)| (*kind, id) == (party.0, &party.1)) {
            posters.push((party.0, party.1, amount, unit));
        }
    }
    let Some(first) = posters.first() else { return Ok(()) };
    let (from_kind, from_id) = match &client {
        Some(client) => (PartyKind::Client, client.as_str()),
        None => (PartyKind::UnknownClient, opportunity_id),
    };
    insert_inferred_link(transaction, opportunity_id, (from_kind, from_id), (first.0, &first.1), None, None, now).await?;
    for pair in posters.windows(2) {
        let (upstream, downstream) = (&pair[0], &pair[1]);
        insert_inferred_link(transaction, opportunity_id, (upstream.0, &upstream.1), (downstream.0, &downstream.1),
            upstream.2, upstream.3.as_deref(), now).await?;
    }
    Ok(())
}

async fn insert_inferred_link(
    transaction: &mut Transaction<'_, Sqlite>, opportunity_id: &str, from: (PartyKind, &str), to: (PartyKind, &str),
    amount: Option<i64>, unit: Option<&str>, now: i64,
) -> Result<()> {
    sqlx::query("INSERT INTO chain_links(id,opportunity_id,from_kind,from_id,to_kind,to_id,rate_amount_inr,rate_unit,inferred,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9)
                 ON CONFLICT(opportunity_id,from_kind,from_id,to_kind,to_id) DO NOTHING")
        .bind(Uuid::new_v4().to_string()).bind(opportunity_id).bind(from.0.as_str()).bind(from.1)
        .bind(to.0.as_str()).bind(to.1).bind(amount).bind(unit).bind(now).execute(&mut **transaction).await?;
    Ok(())
}
