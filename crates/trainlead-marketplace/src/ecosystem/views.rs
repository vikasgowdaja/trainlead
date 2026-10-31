use serde::Serialize;
use sqlx::{Sqlite, SqlitePool, Transaction};
use utoipa::ToSchema;

use super::{Actor, Logistics, PartyKind, Role, deny, rates::{Rate, RateUnit}};
use crate::{Error, Result, audit, clock::Clock};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Canonical {
    pub tech_stack: Vec<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub college: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub mode: Option<String>,
    pub audience: Option<String>,
    pub summary: String,
    pub logistics: Logistics,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PartyView {
    pub kind: PartyKind,
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostingRole { Origin, Repost }

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Contact {
    pub name: Option<String>,
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostingView {
    pub id: String,
    pub role: PostingRole,
    pub poster: PartyView,
    pub channel: String,
    pub posted_at: String,
    pub source_group: Option<String>,
    pub raw_text: String,
    pub rate_text: Option<String>,
    pub rate: Option<Rate>,
    pub payment_date: Option<String>,
    pub logistics: Logistics,
    pub contact: Option<Contact>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChainLinkView {
    pub id: String,
    pub from: PartyView,
    pub to: PartyView,
    /// Present only for the paying party, admins, or a recipient the rate was disclosed to.
    pub rate: Option<Rate>,
    pub inferred: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OpportunityView {
    pub id: String,
    pub status: String,
    pub confidence: f64,
    pub client_id: Option<String>,
    pub client_match: String,
    pub merged_into: Option<String>,
    pub canonical: Canonical,
    pub postings: Vec<PostingView>,
    /// Postings the viewer may not see; vendors learn only that they exist.
    pub other_postings: usize,
    pub chain: Vec<ChainLinkView>,
}

#[derive(sqlx::FromRow)]
struct OpportunityRow {
    status: String, tech_stack: String, start_date: Option<String>, end_date: Option<String>,
    college: Option<String>, city: Option<String>, state: Option<String>, mode: Option<String>,
    audience: Option<String>, summary: String, logistics: String, client_id: Option<String>,
    client_match: String, confidence: f64, merged_into: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PostingRow {
    id: String, posted_by_org_id: Option<String>, external_contact_id: Option<String>, channel: String,
    rate_text: Option<String>, rate_amount_inr: Option<i64>, rate_unit: Option<String>, payment_date: Option<String>,
    logistics: String, contact_name: Option<String>, contact_phone: Option<String>, raw_text: String,
    source_group: Option<String>, posted_at: String,
}

#[derive(sqlx::FromRow)]
struct LinkRow {
    id: String, from_kind: String, from_id: String, to_kind: String, to_id: String,
    rate_amount_inr: Option<i64>, rate_unit: Option<String>, disclosed_to_recipient: bool, inferred: bool,
}

async fn can_view(pool: &SqlitePool, actor: &Actor, id: &str) -> Result<bool> {
    let found: Option<i64> = match actor.role() {
        Role::Admin => return Ok(true),
        Role::Vendor { org_id } => sqlx::query_scalar(
            "SELECT 1 FROM postings WHERE opportunity_id=?1 AND posted_by_org_id=?2
             UNION SELECT 1 FROM chain_links WHERE opportunity_id=?1 AND ((from_kind='vendor' AND from_id=?2) OR (to_kind='vendor' AND to_id=?2))
             LIMIT 1").bind(id).bind(org_id).fetch_optional(pool).await?,
        Role::Trainer { trainer_id } => sqlx::query_scalar(
            "SELECT 1 FROM chain_links WHERE opportunity_id=?1 AND to_kind='trainer' AND to_id=?2 LIMIT 1")
            .bind(id).bind(trainer_id).fetch_optional(pool).await?,
    };
    Ok(found.is_some())
}

fn rate(amount: Option<i64>, unit: Option<&str>) -> Option<Rate> {
    amount.map(|amount_inr| Rate { amount_inr, unit: unit.and_then(RateUnit::parse) })
}

async fn party(transaction: &mut Transaction<'_, Sqlite>, kind: &str, id: &str) -> Result<PartyView> {
    let kind = PartyKind::parse(kind)?;
    let name: Option<String> = match kind {
        PartyKind::Vendor => sqlx::query_scalar("SELECT name FROM organizations WHERE id=?1").bind(id).fetch_optional(&mut **transaction).await?,
        PartyKind::Client => sqlx::query_scalar("SELECT name FROM clients WHERE id=?1").bind(id).fetch_optional(&mut **transaction).await?,
        PartyKind::Trainer => sqlx::query_scalar("SELECT display_name FROM trainers WHERE id=?1").bind(id).fetch_optional(&mut **transaction).await?,
        PartyKind::External => sqlx::query_scalar::<_, Option<String>>("SELECT name FROM external_contacts WHERE id=?1")
            .bind(id).fetch_optional(&mut **transaction).await?.flatten(),
        PartyKind::UnknownClient => return Ok(PartyView { kind, id: None, name: None }),
    };
    Ok(PartyView { kind, id: Some(id.to_string()), name })
}

/// Orders hops upstream to downstream: a hop's depth is the number of hops leading into its payer.
fn order_by_depth(links: &mut [LinkRow]) {
    let depths: Vec<usize> = links.iter().map(|link| {
        let mut payer = (link.from_kind.as_str(), link.from_id.as_str());
        let mut depth = 0;
        while depth < links.len() {
            match links.iter().find(|parent| (parent.to_kind.as_str(), parent.to_id.as_str()) == payer) {
                Some(parent) => { payer = (parent.from_kind.as_str(), parent.from_id.as_str()); depth += 1; }
                None => break,
            }
        }
        depth
    }).collect();
    let mut order: Vec<usize> = (0..links.len()).collect();
    order.sort_by_key(|index| depths[*index]);
    let mut position = vec![0; links.len()];
    for (rank, index) in order.into_iter().enumerate() { position[index] = rank; }
    for start in 0..links.len() {
        while position[start] != start {
            let target = position[start];
            links.swap(start, target);
            position.swap(start, target);
        }
    }
}

fn is_party(actor: &Actor, kind: &str, id: &str) -> bool {
    match actor.role() {
        Role::Vendor { org_id } => kind == "vendor" && id == org_id,
        Role::Trainer { trainer_id } => kind == "trainer" && id == trainer_id,
        Role::Admin => false,
    }
}

/// Party-scoped projection of one opportunity. Every rate or contact shown across parties is audited.
pub async fn opportunity_view(pool: &SqlitePool, clock: &dyn Clock, actor: &Actor, id: &str) -> Result<OpportunityView> {
    let now = clock.now();
    if !can_view(pool, actor, id).await? { return Err(deny(pool, actor, "opportunity:view", "opportunity", id, now).await); }
    let mut transaction = pool.begin().await?;
    let row: Option<OpportunityRow> = sqlx::query_as(
        "SELECT status,tech_stack,start_date,end_date,college,city,state,mode,audience,summary,logistics,
         client_id,client_match,confidence,merged_into FROM opportunities WHERE id=?1")
        .bind(id).fetch_optional(&mut *transaction).await?;
    let row = row.ok_or(Error::NotFound)?;
    if row.merged_into.is_some() && !actor.is_admin() { return Err(Error::NotFound); }
    let actor_id = Some(actor.user_id());

    let posting_rows: Vec<PostingRow> = sqlx::query_as(
        "SELECT id,posted_by_org_id,external_contact_id,channel,rate_text,rate_amount_inr,rate_unit,payment_date,logistics,
         contact_name,contact_phone,raw_text,source_group,posted_at FROM postings WHERE opportunity_id=?1 ORDER BY posted_at, created_at, id")
        .bind(id).fetch_all(&mut *transaction).await?;
    let mut postings = Vec::new();
    let mut other_postings = 0;
    for (index, posting) in posting_rows.into_iter().enumerate() {
        let own = matches!((actor.role(), &posting.posted_by_org_id), (Role::Vendor { org_id }, Some(poster)) if org_id == poster);
        if !(own || actor.is_admin()) {
            if actor.vendor_org().is_some() { other_postings += 1; }
            continue;
        }
        let poster = match (&posting.posted_by_org_id, &posting.external_contact_id) {
            (Some(org), _) => party(&mut transaction, "vendor", org).await?,
            (None, Some(external)) => party(&mut transaction, "external", external).await?,
            (None, None) => PartyView { kind: PartyKind::External, id: None, name: None },
        };
        let has_rate = posting.rate_text.is_some() || posting.rate_amount_inr.is_some();
        let has_contact = posting.contact_name.is_some() || posting.contact_phone.is_some();
        if !own && has_rate { audit::append(&mut transaction, actor_id, "rate:view", "posting", &posting.id, now, false).await?; }
        if !own && has_contact { audit::append(&mut transaction, actor_id, "contact:view", "posting", &posting.id, now, false).await?; }
        postings.push(PostingView {
            role: if index == 0 { PostingRole::Origin } else { PostingRole::Repost },
            poster, channel: posting.channel, posted_at: posting.posted_at, source_group: posting.source_group,
            raw_text: posting.raw_text, rate_text: posting.rate_text,
            rate: rate(posting.rate_amount_inr, posting.rate_unit.as_deref()), payment_date: posting.payment_date,
            logistics: serde_json::from_str(&posting.logistics)?,
            contact: has_contact.then_some(Contact { name: posting.contact_name, phone: posting.contact_phone }),
            id: posting.id,
        });
    }

    let mut link_rows: Vec<LinkRow> = sqlx::query_as(
        "SELECT id,from_kind,from_id,to_kind,to_id,rate_amount_inr,rate_unit,disclosed_to_recipient,inferred
         FROM chain_links WHERE opportunity_id=?1 ORDER BY created_at, id")
        .bind(id).fetch_all(&mut *transaction).await?;
    order_by_depth(&mut link_rows);
    let mut chain = Vec::new();
    for link in link_rows {
        let payer = is_party(actor, &link.from_kind, &link.from_id);
        let recipient = is_party(actor, &link.to_kind, &link.to_id);
        if !(actor.is_admin() || payer || recipient) { continue; }
        let visible = actor.is_admin() || payer || (recipient && link.disclosed_to_recipient);
        let shown = if visible { rate(link.rate_amount_inr, link.rate_unit.as_deref()) } else { None };
        if shown.is_some() && !payer { audit::append(&mut transaction, actor_id, "rate:view", "chain_link", &link.id, now, false).await?; }
        chain.push(ChainLinkView {
            from: party(&mut transaction, &link.from_kind, &link.from_id).await?,
            to: party(&mut transaction, &link.to_kind, &link.to_id).await?,
            rate: shown, inferred: link.inferred, id: link.id,
        });
    }
    transaction.commit().await?;
    Ok(OpportunityView {
        id: id.to_string(), status: row.status, confidence: row.confidence, client_id: row.client_id,
        client_match: row.client_match, merged_into: row.merged_into,
        canonical: Canonical {
            tech_stack: serde_json::from_str(&row.tech_stack)?, start_date: row.start_date, end_date: row.end_date,
            college: row.college, city: row.city, state: row.state, mode: row.mode, audience: row.audience,
            summary: row.summary, logistics: serde_json::from_str(&row.logistics)?,
        },
        postings, other_postings, chain,
    })
}
