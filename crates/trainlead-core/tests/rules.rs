#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use trainlead_core::{
    Error, Result,
    dedup::DedupThresholds,
    extraction::{ExtractionRequest, LlmClient},
    models::{Message, Mode, Provision, Requirement},
    pipeline::Pipeline,
    rules::{FallbackExtractor, RuleExtractor},
    source::SourceBatch,
    store::Store,
};

fn date(year: i32, month: u32, day: u32) -> Option<NaiveDate> { NaiveDate::from_ymd_opt(year, month, day) }
fn today() -> NaiveDate { NaiveDate::from_ymd_opt(2026, 10, 31).unwrap() }

fn extract(text: &str, sender: Option<&str>) -> Vec<Requirement> {
    RuleExtractor::new().unwrap().requirements(text, sender, today())
}

#[test]
fn reads_the_sample_angular_requirement() {
    let found = extract("AngularJS, Mysuru, 2-7 Nov, 6th sem engg, TFA provided, payment 25 Nov", None);
    assert_eq!(found.len(), 1);
    let requirement = &found[0];
    assert_eq!(requirement.tech_stack, vec!["AngularJS"]);
    assert_eq!((requirement.city.as_deref(), requirement.state.as_deref()), (Some("Mysuru"), Some("Karnataka")));
    assert_eq!((requirement.start_date, requirement.end_date, requirement.duration_days), (date(2026, 11, 2), date(2026, 11, 7), Some(6)));
    assert_eq!(requirement.audience.as_deref(), Some("6th sem Engg"));
    assert_eq!([requirement.travel, requirement.food, requirement.accommodation], [Provision::Provided; 3]);
    assert_eq!(requirement.logistics_note.as_deref(), Some("TFA provided"));
    assert_eq!(requirement.payment_date, date(2026, 11, 25));
    assert_eq!(requirement.summary, "AngularJS training in Mysuru, 2 Nov to 7 Nov 2026");
}

#[test]
fn reads_a_pasted_whatsapp_hiring_post_with_header_and_phone() {
    let text = "[11:41 am, 31/10/2026] +91 91319 72687: 📢 Urgent Hiring - RAG Trainer\n\n\
                We are looking for an experienced RAG (Retrieval-Augmented Generation) Trainer.\n\n\
                📍 Location: Lucknow\n📅 Start Date: 1 November 2026\nDuration: 5 days\nMode: Offline\n\
                Interested trainers can DM.";
    let found = extract(text, None);
    assert_eq!(found.len(), 1);
    let requirement = &found[0];
    assert!(requirement.tech_stack.contains(&"RAG".to_string()), "{:?}", requirement.tech_stack);
    assert_eq!(requirement.city.as_deref(), Some("Lucknow"));
    assert_eq!(requirement.start_date, date(2026, 11, 1), "the 31/10 WhatsApp header is not the training date");
    assert_eq!(requirement.duration_days, Some(5));
    assert_eq!(requirement.mode, Some(Mode::Offline));
    assert_eq!(requirement.vendor_phone.as_deref(), Some("919131972687"));
}

fn sorted(mut values: Vec<String>) -> Vec<String> { values.sort(); values }

#[test]
fn reads_contact_details_and_no_tfa() {
    let text = "Requirement: Python and Data Science trainer for B.Tech students\nVenue: Hyderabad\nDates: 10th to 14th Nov\n\
                Rate: 8k per day\nLocal trainers are preferred. No TFA.\n\nPlease DM if interested.\n\nZoya Ahmed\n8688394642";
    let requirement = &extract(text, None)[0];
    assert_eq!(sorted(requirement.tech_stack.clone()), vec!["Data Science", "Python"]);
    assert_eq!([requirement.travel, requirement.food, requirement.accommodation], [Provision::NotProvided; 3]);
    assert_eq!(requirement.rate.as_deref(), Some("8k per day"));
    assert_eq!(requirement.vendor_name.as_deref(), Some("Zoya Ahmed"));
    assert_eq!(requirement.vendor_phone.as_deref(), Some("918688394642"));
    assert_eq!(requirement.audience.as_deref(), Some("B.TECH students"));
}

#[test]
fn finds_colleges_logistics_lines_and_online_mode() {
    let text = "Workshop at Bengaluru Institute of Technology, Bengaluru on React and Node.js\n\
                Nov 18 to 20, online via Zoom\nTravel and stay not provided, lunch provided\nRs 1200 per hour";
    let requirement = &extract(text, None)[0];
    assert_eq!(requirement.college.as_deref(), Some("Bengaluru Institute of Technology"));
    assert_eq!(sorted(requirement.tech_stack.clone()), vec!["Node.js", "React"]);
    assert_eq!(requirement.mode, Some(Mode::Online));
    assert_eq!((requirement.travel, requirement.accommodation), (Provision::NotProvided, Provision::NotProvided));
    assert_eq!(requirement.rate.as_deref(), Some("Rs 1200 per hour"));
}

#[test]
fn one_message_listing_three_cities_becomes_three_requirements() {
    let found = extract("Java training requirement in Pune, Nagpur and Nashik from 10-14 Dec", None);
    let cities: Vec<_> = found.iter().map(|requirement| requirement.city.clone().unwrap_or_default()).collect();
    assert_eq!(cities, ["Pune", "Nagpur", "Nashik"]);
}

#[test]
fn skips_greetings_trainer_availability_and_job_ads() {
    assert!(extract("Good morning all trainers, have a great day", None).is_empty());
    assert!(extract("I am a certified Java trainer, available for training assignments in Pune", None).is_empty());
    assert!(extract("Hiring Java developer, full time, CTC 8 LPA, Bengaluru", None).is_empty());
    assert!(extract("TeacherCool.com is Looking for a Java Big Data Developer! Spring Boot, Spark, AWS EMR. \
                     Interested developers or referrals can share profiles", None).is_empty());
}

#[test]
fn pasted_whatsapp_copy_splits_into_separate_requirements() -> trainlead_core::Result<()> {
    let pasted = "[11:41 am, 31/10/2026] +91 91319 72687: Urgent Hiring - RAG Trainer\n📍 Location: Lucknow\n\
                  📅 Start Date: 1 November 2026\n✈️ TFA: Provided\n\
                  [2:37 pm, 31/10/2026] Sudhir Vendor Cognizant: Urgent Requirement - Soft Skills Trainers\n\
                  📍 College: Balaji College, Jaipur, Rajasthan\n📅 Training Start Date: 1st December 2026\n💰 Commercial: ₹3,000 per day";
    let messages = trainlead_core::parser::parse_export(pasted, "Pasted")?;
    assert_eq!(messages.len(), 2);
    let rag = &extract(&messages[0].raw_text, messages[0].sender.as_deref())[0];
    assert_eq!((rag.city.as_deref(), rag.start_date, rag.tech_stack.clone()), (Some("Lucknow"), date(2026, 11, 1), vec!["RAG".to_string()]));
    assert_eq!(rag.vendor_phone.as_deref(), Some("919131972687"));
    let balaji = &extract(&messages[1].raw_text, messages[1].sender.as_deref())[0];
    assert_eq!((balaji.college.as_deref(), balaji.city.as_deref()), (Some("Balaji College"), Some("Jaipur")));
    assert_eq!((balaji.start_date, balaji.rate.as_deref()), (date(2026, 12, 1), Some("₹3,000 per day")));
    assert_eq!(balaji.vendor_name.as_deref(), Some("Sudhir Vendor Cognizant"));
    Ok(())
}

#[test]
fn uses_the_sender_when_the_text_has_no_contact() {
    let requirement = &extract("Selenium training, Chennai, 3-5 Dec", Some("Ravi Kumar +91 98450 12345"))[0];
    assert_eq!(requirement.vendor_name.as_deref(), Some("Ravi Kumar"));
    assert_eq!(requirement.vendor_phone.as_deref(), Some("919845012345"));
}

struct Failing(Error);

#[async_trait]
impl LlmClient for Failing {
    async fn extract(&self, _: &ExtractionRequest) -> Result<String> {
        Err(match &self.0 { Error::ApiStatus(code) => Error::ApiStatus(*code), _ => Error::Validation("down".into()) })
    }
}

struct Garbage;

#[async_trait]
impl LlmClient for Garbage {
    async fn extract(&self, _: &ExtractionRequest) -> Result<String> { Ok("not json".into()) }
}

fn messages() -> SourceBatch {
    SourceBatch::Messages(vec![Message {
        source_group: "Trainers".into(), sender: Some("Asha".into()), timestamp: "2026-10-31T09:00:00+05:30".into(),
        raw_text: "Training requirement: AngularJS, Mysuru, 2-7 Nov, TFA provided".into(),
    }])
}

#[tokio::test]
async fn falls_back_to_rules_when_groq_fails_or_returns_garbage() -> Result<()> {
    let rules = Arc::new(RuleExtractor::new()?);
    for primary in [Arc::new(Failing(Error::ApiStatus(404))) as Arc<dyn LlmClient>, Arc::new(Garbage)] {
        let fallback = Arc::new(FallbackExtractor::new(Some(primary), rules.clone()));
        let mut store = Store::in_memory(DedupThresholds::default())?;
        let report = Pipeline::new(fallback.clone())?.ingest(messages(), &mut store, today()).await?;
        assert_eq!((report.found, report.needs_review), (1, 0));
        assert!(fallback.engine().starts_with("on-device rules (Groq failed"), "{}", fallback.engine());
        assert_eq!(store.clusters()?[0].primary.requirement.tech_stack, vec!["AngularJS"]);
    }
    let local_only = Arc::new(FallbackExtractor::<Failing>::new(None, rules));
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let report = Pipeline::new(local_only.clone())?.ingest(messages(), &mut store, today()).await?;
    assert_eq!(report.found, 1);
    assert_eq!(local_only.engine(), "on-device rules");
    Ok(())
}
