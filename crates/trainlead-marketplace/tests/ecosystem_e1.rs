#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use chrono::NaiveDate;
use sqlx::SqlitePool;
use trainlead_core::{dedup::DedupThresholds, models::{Cluster, Message, Mode, Provision, Requirement}, store::Store};
use trainlead_marketplace::{
    Error, Result,
    clock::Clock,
    db,
    ecosystem::{
        Actor, PartyKind, events,
        chain::{Recipient, add_link},
        clients::{ClientKind, ClientMatch, NewClient, confirm_client, create_client, resolve_client},
        ingest::import_clusters,
        opportunity::{OpportunityStatus, merge, transition, unmerge},
        rates::{Rate, RateUnit},
        views::{PostingRole, opportunity_view},
    },
};

struct FixedClock(i64);
impl Clock for FixedClock { fn now(&self) -> i64 { self.0 } }
const CLOCK: FixedClock = FixedClock(1_793_500_000);

async fn user(pool: &SqlitePool, id: &str, admin: bool) {
    sqlx::query("INSERT INTO users(id,email,created_at) VALUES(?1,?2,0)").bind(id).bind(format!("{id}@example.test"))
        .execute(pool).await.unwrap();
    if admin {
        sqlx::query("INSERT INTO user_roles(user_id,role) VALUES(?1,'Admin')").bind(id).execute(pool).await.unwrap();
    }
}

async fn vendor(pool: &SqlitePool, name: &str, phone: &str) -> (String, Actor) {
    let org = format!("org-{name}");
    let member = format!("user-{name}");
    user(pool, &member, false).await;
    sqlx::query("INSERT INTO organizations(id,name,kind,status,created_at) VALUES(?1,?2,'vendor','active',0)")
        .bind(&org).bind(name).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO org_members(org_id,user_id,role) VALUES(?1,?2,'VendorOwner')").bind(&org).bind(&member).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vendor_contacts(phone,org_id) VALUES(?1,?2)").bind(phone).bind(&org).execute(pool).await.unwrap();
    let actor = Actor::vendor(pool, &member, &org).await.unwrap();
    (org, actor)
}

async fn trainer(pool: &SqlitePool, name: &str) -> (String, Actor) {
    let id = format!("trainer-{name}");
    let member = format!("user-{name}");
    user(pool, &member, false).await;
    sqlx::query("INSERT INTO trainers(id,user_id,display_name,created_at) VALUES(?1,?2,?3,0)")
        .bind(&id).bind(&member).bind(name).execute(pool).await.unwrap();
    (id.clone(), Actor::trainer(pool, &member).await.unwrap())
}

async fn admin(pool: &SqlitePool) -> Actor {
    user(pool, "ops", true).await;
    Actor::admin(pool, "ops").await.unwrap()
}

fn date(month: u32, day: u32) -> NaiveDate { NaiveDate::from_ymd_opt(2026, month, day).unwrap() }

fn angular_mysuru(vendor: &str, phone: &str, rate: &str) -> Requirement {
    Requirement {
        tech_stack: vec!["AngularJS".into()], start_date: Some(date(11, 2)), end_date: Some(date(11, 7)), duration_days: Some(6),
        college: Some("Mysuru Engineering College".into()), city: Some("Mysuru".into()), state: Some("Karnataka".into()),
        mode: Some(Mode::Offline), audience: Some("6th sem Engg".into()), travel: Provision::Provided, food: Provision::Provided,
        accommodation: Provision::Provided, logistics_note: Some("TFA provided".into()), rate: Some(rate.into()),
        payment_date: Some(date(11, 25)), vendor_name: Some(vendor.into()), vendor_company: None,
        vendor_phone: Some(phone.into()), summary: "AngularJS training for 6th sem engineering in Mysuru".into(),
    }
}

fn react_bengaluru(vendor: &str, phone: &str) -> Requirement {
    Requirement {
        tech_stack: vec!["React".into()], start_date: Some(date(12, 8)), end_date: Some(date(12, 12)), duration_days: Some(5),
        college: Some("Bengaluru Institute of Technology".into()), city: Some("Bengaluru".into()), state: None,
        mode: Some(Mode::Online), audience: None, travel: Provision::NotProvided, food: Provision::Unknown,
        accommodation: Provision::NotProvided, logistics_note: None, rate: Some("1200 per hour".into()), payment_date: None,
        vendor_name: Some(vendor.into()), vendor_company: None, vendor_phone: Some(phone.into()),
        summary: "React bootcamp in Bengaluru".into(),
    }
}

/// Runs the posts through the real ingest dedup store so the importer sees genuine clusters.
fn clusters(posts: Vec<(&str, &str, &str, Requirement)>) -> Vec<Cluster> {
    let mut store = Store::in_memory(DedupThresholds::default()).unwrap();
    let messages: Vec<Message> = posts.iter().map(|(group, timestamp, raw, requirement)| Message {
        source_group: (*group).into(), sender: requirement.vendor_name.clone(), timestamp: (*timestamp).into(), raw_text: (*raw).into(),
    }).collect();
    let ids = store.record_messages(&messages).unwrap();
    let records: Vec<_> = ids.iter().zip(messages).zip(posts).map(|((id, message), (_, _, _, requirement))| (*id, message, requirement)).collect();
    store.save_batch(&ids, &records).unwrap();
    store.clusters().unwrap()
}

fn three_vendor_posts() -> Vec<(&'static str, &'static str, &'static str, Requirement)> {
    vec![
        ("Trainers Karnataka", "2026-10-30T09:00:00+05:30",
         "Requirement: AngularJS trainer, Mysuru Engineering College, 2-7 Nov, 6th sem engg, TFA provided, 9000/day, payment 25 Nov",
         angular_mysuru("Asha", "919800000001", "₹9,000/day")),
        ("Freelance Trainers India", "2026-10-30T11:00:00+05:30",
         "Urgent!! Angular JS trainer needed at Mysuru Engg college Nov 2 to 7. TFA. 8k per day. Pay 25th Nov",
         angular_mysuru("Bala", "919800000002", "8k per day")),
        ("IT Trainers Network", "2026-10-30T13:00:00",
         "Angular training Mysuru 2nd-7th November, sixth semester, travel food stay covered, Rs 7000/day",
         angular_mysuru("Chitra", "919800000003", "Rs 7000/day")),
    ]
}

async fn audit_count(pool: &SqlitePool, actor: &str, action: &str, outcome: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE actor_id=?1 AND action=?2 AND outcome=?3")
        .bind(actor).bind(action).bind(outcome).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn three_vendor_posts_become_one_opportunity_with_scoped_rates() -> Result<()> {
    let pool = db::connect("sqlite::memory:").await?;
    let ops = admin(&pool).await;
    let (org_a, _) = vendor(&pool, "Asha Trainings", "919800000001").await;
    let (org_b, vendor_b) = vendor(&pool, "Bala Skills", "919800000002").await;
    let (org_c, vendor_c) = vendor(&pool, "Chitra Learning", "919800000003").await;
    let (_, outsider) = vendor(&pool, "Dev Academy", "919800000004").await;
    let (trainer_id, trainer_actor) = trainer(&pool, "Ravi").await;
    let (_, other_trainer) = trainer(&pool, "Meena").await;

    let clusters = clusters(three_vendor_posts());
    assert_eq!(clusters.len(), 1, "core dedup must cluster the three reposts");
    let report = import_clusters(&pool, &CLOCK, &clusters).await?;
    assert_eq!((report.opportunities_created, report.postings_created), (1, 3));
    let again = import_clusters(&pool, &CLOCK, &clusters).await?;
    assert_eq!((again.opportunities_created, again.postings_created, again.postings_skipped), (0, 0, 3));
    let opportunity: String = sqlx::query_scalar("SELECT id FROM opportunities").fetch_one(&pool).await?;

    let full = opportunity_view(&pool, &CLOCK, &ops, &opportunity).await?;
    assert_eq!(full.postings.len(), 3);
    assert_eq!(full.postings[0].role, PostingRole::Origin);
    assert_eq!(full.postings[0].poster.id.as_deref(), Some(org_a.as_str()));
    assert!(full.postings[1..].iter().all(|posting| posting.role == PostingRole::Repost));
    let hops: Vec<_> = full.chain.iter().map(|link| (link.from.kind, link.to.id.clone(), link.rate.map(|rate| rate.amount_inr))).collect();
    assert_eq!(hops, vec![
        (PartyKind::UnknownClient, Some(org_a.clone()), None),
        (PartyKind::Vendor, Some(org_b.clone()), Some(9000)),
        (PartyKind::Vendor, Some(org_c.clone()), Some(8000)),
    ]);
    assert_eq!(audit_count(&pool, "ops", "rate:view", "success").await, 5);
    assert_eq!(audit_count(&pool, "ops", "contact:view", "success").await, 3);

    let seen_by_b = opportunity_view(&pool, &CLOCK, &vendor_b, &opportunity).await?;
    assert_eq!(seen_by_b.postings.len(), 1);
    assert_eq!(seen_by_b.postings[0].rate, Some(Rate { amount_inr: 8000, unit: Some(RateUnit::Day) }));
    assert_eq!(seen_by_b.other_postings, 2);
    let incoming = seen_by_b.chain.iter().find(|link| link.to.id.as_deref() == Some(org_b.as_str())).unwrap();
    assert_eq!(incoming.rate, None, "B must not see the rate A owns");
    let json = serde_json::to_string(&seen_by_b)?;
    for leak in ["9000", "9,000", "919800000001", "7000", "919800000003"] { assert!(!json.contains(leak), "vendor B saw {leak}"); }

    add_link(&pool, &CLOCK, &vendor_c, &opportunity, &Recipient::Trainer(trainer_id.clone()),
        Some(Rate { amount_inr: 6000, unit: Some(RateUnit::Day) }), true).await?;
    let seen_by_trainer = opportunity_view(&pool, &CLOCK, &trainer_actor, &opportunity).await?;
    assert!(seen_by_trainer.postings.is_empty());
    assert_eq!(seen_by_trainer.other_postings, 0);
    assert_eq!(seen_by_trainer.chain.len(), 1);
    assert_eq!(seen_by_trainer.chain[0].rate.map(|rate| rate.amount_inr), Some(6000));
    assert_eq!(seen_by_trainer.canonical.logistics.note.as_deref(), Some("TFA provided"));
    let json = serde_json::to_string(&seen_by_trainer)?;
    for leak in ["9000", "8000", "7000", "9198000000"] { assert!(!json.contains(leak), "trainer saw {leak}"); }
    assert_eq!(audit_count(&pool, trainer_actor.user_id(), "rate:view", "success").await, 1);

    for denied in [&outsider, &other_trainer] {
        assert!(matches!(opportunity_view(&pool, &CLOCK, denied, &opportunity).await, Err(Error::Forbidden)));
        assert_eq!(audit_count(&pool, denied.user_id(), "opportunity:view", "denied").await, 1);
    }
    assert!(matches!(add_link(&pool, &CLOCK, &outsider, &opportunity, &Recipient::Vendor(org_a), None, false).await, Err(Error::Forbidden)));
    Ok(())
}

async fn snapshot(pool: &SqlitePool) -> (Vec<(String, String, Option<String>, Option<String>, String)>, Vec<(String, String, String, String, String, String, Option<i64>)>) {
    let postings = sqlx::query_as("SELECT id,opportunity_id,posted_by_org_id,rate_text,posted_at FROM postings ORDER BY id")
        .fetch_all(pool).await.unwrap();
    let links = sqlx::query_as("SELECT id,opportunity_id,from_kind,from_id,to_kind,to_id,rate_amount_inr FROM chain_links ORDER BY id")
        .fetch_all(pool).await.unwrap();
    (postings, links)
}

#[tokio::test]
async fn merge_then_unmerge_restores_every_posting_and_link() -> Result<()> {
    let pool = db::connect("sqlite::memory:").await?;
    let ops = admin(&pool).await;
    let (_, vendor_a) = vendor(&pool, "Asha Trainings", "919800000001").await;
    vendor(&pool, "Bala Skills", "919800000002").await;
    let mut posts = three_vendor_posts();
    posts.push(("Trainers Karnataka", "2026-10-31T10:00:00+05:30", "React bootcamp, Bengaluru Institute of Technology, 8-12 Dec, online, 1200/hour",
        react_bengaluru("Asha", "919800000001")));
    import_clusters(&pool, &CLOCK, &clusters(posts)).await?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM opportunities ORDER BY start_date").fetch_all(&pool).await?;
    assert_eq!(ids.len(), 2);
    let before = snapshot(&pool).await;

    assert!(matches!(merge(&pool, &CLOCK, &vendor_a, &ids[0], &ids[1]).await, Err(Error::Forbidden)));
    let merge_id = merge(&pool, &CLOCK, &ops, &ids[0], &ids[1]).await?;
    let on_target: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE opportunity_id=?1").bind(&ids[0]).fetch_one(&pool).await?;
    assert_eq!(on_target, 4);
    assert!(matches!(opportunity_view(&pool, &CLOCK, &vendor_a, &ids[1]).await, Err(Error::NotFound)));
    assert!(matches!(merge(&pool, &CLOCK, &ops, &ids[0], &ids[1]).await, Err(Error::Conflict(_))));

    unmerge(&pool, &CLOCK, &ops, &merge_id).await?;
    assert_eq!(snapshot(&pool).await, before);
    assert!(matches!(unmerge(&pool, &CLOCK, &ops, &merge_id).await, Err(Error::Conflict(_))));
    let history: Vec<String> = events(&pool, "opportunity", &ids[1]).await?.into_iter().map(|event| event.event).collect();
    assert_eq!(history.last().map(String::as_str), Some("unmerged"));
    assert!(history.contains(&"merged_into".to_string()));
    Ok(())
}

#[tokio::test]
async fn client_resolution_confirms_exact_suggests_close_and_learns_aliases() -> Result<()> {
    let pool = db::connect("sqlite::memory:").await?;
    let ops = admin(&pool).await;
    let (_, vendor_a) = vendor(&pool, "Asha Trainings", "919800000001").await;
    let new_client = || NewClient { name: "Mysuru Engineering College".into(), kind: ClientKind::College,
        city: Some("Mysuru".into()), state: Some("Karnataka".into()), contacts: vec![] };
    assert!(matches!(create_client(&pool, &CLOCK, &vendor_a, new_client()).await, Err(Error::Forbidden)));
    let client = create_client(&pool, &CLOCK, &ops, new_client()).await?;

    assert_eq!(resolve_client(&pool, "Mysore Engg College", Some("Mysore")).await?, ClientMatch::Confirmed { client_id: client.clone() });
    assert!(matches!(resolve_client(&pool, "Mysuru Engineering College VV Mohalla", Some("Mysuru")).await?,
        ClientMatch::Suggested { ref client_id, .. } if *client_id == client));
    assert_eq!(resolve_client(&pool, "Mysuru Institute of Technology", Some("Mysuru")).await?, ClientMatch::None);
    assert_eq!(resolve_client(&pool, "Mysuru Engineering College VV Mohalla", Some("Chennai")).await?, ClientMatch::None);

    let mut uncertain = angular_mysuru("Asha", "919800000001", "9000/day");
    uncertain.college = Some("Mysuru Engineering College VV Mohalla".into());
    import_clusters(&pool, &CLOCK, &clusters(vec![("Trainers", "2026-10-30T09:00:00+05:30", "AngularJS Mysuru 2-7 Nov", uncertain)])).await?;
    let (opportunity, matched): (String, String) = sqlx::query_as("SELECT id, client_match FROM opportunities").fetch_one(&pool).await?;
    assert_eq!(matched, "suggested");
    confirm_client(&pool, &CLOCK, &ops, &opportunity, &client).await?;
    let view = opportunity_view(&pool, &CLOCK, &ops, &opportunity).await?;
    assert_eq!((view.client_id.as_deref(), view.client_match.as_str()), (Some(client.as_str()), "confirmed"));
    assert_eq!(view.chain[0].from.kind, PartyKind::Client);
    assert_eq!(resolve_client(&pool, "Mysuru Engineering College VV Mohalla", Some("Mysuru")).await?, ClientMatch::Confirmed { client_id: client });
    Ok(())
}

#[tokio::test]
async fn opportunity_lifecycle_rejects_illegal_transitions_and_logs_events() -> Result<()> {
    let pool = db::connect("sqlite::memory:").await?;
    let ops = admin(&pool).await;
    let (_, vendor_a) = vendor(&pool, "Asha Trainings", "919800000001").await;
    import_clusters(&pool, &CLOCK, &clusters(three_vendor_posts())).await?;
    let id: String = sqlx::query_scalar("SELECT id FROM opportunities").fetch_one(&pool).await?;
    assert!(matches!(transition(&pool, &CLOCK, &ops, &id, OpportunityStatus::Open).await, Err(Error::Conflict(_))));
    assert!(matches!(transition(&pool, &CLOCK, &vendor_a, &id, OpportunityStatus::Verified).await, Err(Error::Forbidden)));
    transition(&pool, &CLOCK, &ops, &id, OpportunityStatus::Verified).await?;
    transition(&pool, &CLOCK, &ops, &id, OpportunityStatus::Open).await?;
    transition(&pool, &CLOCK, &ops, &id, OpportunityStatus::Cancelled).await?;
    assert!(matches!(transition(&pool, &CLOCK, &ops, &id, OpportunityStatus::Open).await, Err(Error::Conflict(_))));
    let changes: Vec<_> = events(&pool, "opportunity", &id).await?.into_iter()
        .filter(|event| event.event == "status_changed").map(|event| event.to_state.unwrap_or_default()).collect();
    assert_eq!(changes, ["Verified", "Open", "Cancelled"]);
    Ok(())
}
