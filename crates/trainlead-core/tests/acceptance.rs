#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::json;
use trainlead_core::{
    Error, Result,
    dedup::DedupThresholds,
    extraction::{
        ExtractionItem, ExtractionOutput, ExtractionRequest, LlmClient, decode_output,
        system_prompt,
    },
    models::{DuplicateKind, Message, Mode, Provision, Requirement},
    pipeline::Pipeline,
    source::{ExportSource, Screenshot, SourceBatch},
    store::Store,
};

#[derive(Clone)]
struct MockLlm {
    responses: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<ExtractionRequest>>>,
}

impl MockLlm {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl LlmClient for MockLlm {
    async fn extract(&self, request: &ExtractionRequest) -> Result<String> {
        self.requests.lock().unwrap().push(request.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| Error::Validation("unexpected mock call".into()))
    }
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
}
fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
}

fn requirement(city: &str) -> Requirement {
    Requirement {
        tech_stack: vec!["Angular JS".into()],
        start_date: Some(date(2)),
        end_date: Some(date(7)),
        duration_days: Some(6),
        college: Some("Mysuru Engineering College".into()),
        city: Some(city.into()),
        state: Some("Karnataka".into()),
        mode: Some(Mode::Offline),
        audience: Some("6th sem Engg".into()),
        travel: Provision::Provided,
        food: Provision::Provided,
        accommodation: Provision::Provided,
        logistics_note: Some("TFA provided".into()),
        rate: None,
        payment_date: Some(date(25)),
        vendor_name: Some("Alice".into()),
        vendor_company: None,
        vendor_phone: Some("919876543210".into()),
        summary: "AngularJS training, Mysuru, 2-7 Nov".into(),
    }
}

fn message(group: &str, sender: &str, raw_text: &str) -> Message {
    Message {
        source_group: group.into(),
        sender: Some(sender.into()),
        timestamp: "2026-10-31T09:00:00+05:30".into(),
        raw_text: raw_text.into(),
    }
}

fn output(requirements: Vec<(usize, Requirement)>) -> String {
    serde_json::to_string(&ExtractionOutput {
        requirements: requirements
            .into_iter()
            .map(|(message_index, requirement)| ExtractionItem {
                message_index,
                requirement,
                raw_text: None,
            })
            .collect(),
    })
    .unwrap()
}

#[tokio::test]
async fn exact_forward_similar_vendor_repost_and_different_city_date() -> Result<()> {
    let first = requirement("Mysuru");
    let mut repost = first.clone();
    repost.vendor_name = Some("Bob".into());
    repost.vendor_company = Some("Other Vendor".into());
    repost.vendor_phone = Some("919999999999".into());
    let mut different = requirement("Bengaluru");
    different.start_date = Some(date(12));
    different.end_date = Some(date(17));
    let mock = MockLlm::new(vec![output(vec![
        (0, first.clone()),
        (1, first),
        (2, repost),
        (3, different),
    ])]);
    let pipeline = Pipeline::new(mock)?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let raw = "Training requirement AngularJS at Mysuru Engineering College, Mysuru, 2-7 Nov, TFA provided";
    let report = pipeline.ingest(SourceBatch::Messages(vec![
        message("Vendors A", "Alice", raw), message("Vendors B", "Carol", raw),
        message("Vendors C", "Bob", "College workshop: seeking Angular JS instructor in Mysuru for November 2 through 7. Travel, meals and room covered."),
        message("Vendors D", "Dan", "Training requirement AngularJS Bengaluru 12-17 Nov"),
    ]), &mut store, today()).await?;
    assert_eq!(
        (report.found, report.new, report.exact, report.similar),
        (4, 2, 1, 1)
    );
    let clusters = store.clusters()?;
    assert_eq!(clusters.len(), 2);
    let repeated = clusters.iter().find(|cluster| cluster.seen == 3).unwrap();
    assert_eq!(
        (repeated.groups, repeated.exact, repeated.similar),
        (3, 1, 1)
    );
    assert_eq!(
        repeated.primary.requirement.vendor_name.as_deref(),
        Some("Bob")
    );
    assert_eq!(repeated.sources[1].message.sender.as_deref(), Some("Carol"));
    assert_eq!(
        repeated.sources[2].requirement.vendor_phone.as_deref(),
        Some("919999999999")
    );
    assert_eq!(store.messages_by_status("processed")?.len(), 4);
    Ok(())
}

#[tokio::test]
async fn three_cities_in_one_message_produce_three_requirements() -> Result<()> {
    let mut requirements = Vec::new();
    for city in ["Mysuru", "Bengaluru", "Chennai"] {
        let mut record = requirement(city);
        record.college = None;
        record.start_date = None;
        record.end_date = None;
        requirements.push((0, record));
    }
    let pipeline = Pipeline::new(MockLlm::new(vec![output(requirements)]))?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let report = pipeline.ingest(SourceBatch::Messages(vec![message("Vendors", "Alice",
        "Training requirements AngularJS in Mysuru, Bengaluru and Chennai; dates to be confirmed")]), &mut store, today()).await?;
    assert_eq!((report.found, report.new), (3, 3));
    assert_eq!(store.clusters()?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn invalid_json_reasked_once_then_durably_flagged_for_review() -> Result<()> {
    let mock = MockLlm::new(vec!["```not json```".into(), "{bad json}".into()]);
    let pipeline = Pipeline::new(mock.clone())?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let original = message("Vendors", "Alice", "Training requirement AngularJS Mysuru");
    let report = pipeline
        .ingest(
            SourceBatch::Messages(vec![original.clone()]),
            &mut store,
            today(),
        )
        .await?;
    assert_eq!((report.found, report.needs_review), (0, 1));
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].correction);
    assert!(requests[1].correction);
    let review = store.messages_by_status("needs_review")?;
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].message, original);
    assert!(review[0].review_reason.is_some());
    assert!(store.clusters()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn invalid_json_can_be_corrected_without_partial_insertions() -> Result<()> {
    let mut invalid = serde_json::to_value(ExtractionOutput {
        requirements: vec![ExtractionItem {
            message_index: 0,
            requirement: requirement("Mysuru"),
            raw_text: None,
        }],
    })?;
    invalid["requirements"][0]["requirement"]["travel"] = json!("free");
    let mock = MockLlm::new(vec![
        invalid.to_string(),
        output(vec![(0, requirement("Mysuru"))]),
    ]);
    let pipeline = Pipeline::new(mock.clone())?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let report = pipeline
        .ingest(
            SourceBatch::Messages(vec![message("A", "Alice", "Training AngularJS")]),
            &mut store,
            today(),
        )
        .await?;
    assert_eq!((report.new, report.needs_review), (1, 0));
    assert_eq!(mock.requests.lock().unwrap().len(), 2);
    assert_eq!(store.clusters()?[0].seen, 1);
    Ok(())
}

#[tokio::test]
async fn angularjs_sample_retains_dates_logistics_payment_and_source() -> Result<()> {
    let raw = "AngularJS, Mysuru, 2-7 Nov, 6th sem engg, TFA provided, payment 25 Nov";
    let mock = MockLlm::new(vec![output(vec![(0, requirement("Mysuru"))])]);
    let pipeline = Pipeline::new(mock.clone())?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let mut source = ExportSource {
        source_group: "Training Vendors".into(),
        text: format!("31/10/26, 9:00 am - Alice: Training requirement\n{raw}"),
    };
    pipeline
        .ingest_source(&mut source, &mut store, today())
        .await?;
    let lead = store.clusters()?.remove(0).primary;
    assert_eq!(lead.requirement.tech_stack, vec!["AngularJS"]);
    assert_eq!(
        (
            lead.requirement.start_date,
            lead.requirement.end_date,
            lead.requirement.duration_days
        ),
        (Some(date(2)), Some(date(7)), Some(6))
    );
    assert_eq!(lead.requirement.payment_date, Some(date(25)));
    assert_eq!(
        [
            lead.requirement.travel,
            lead.requirement.food,
            lead.requirement.accommodation
        ],
        [Provision::Provided; 3]
    );
    assert_eq!(lead.requirement.audience.as_deref(), Some("6th sem Engg"));
    assert_eq!(lead.message.source_group, "Training Vendors");
    assert!(lead.message.raw_text.contains(raw));
    assert_eq!(mock.requests.lock().unwrap()[0].today, today());
    assert!(system_prompt(today(), false).contains("Today is 2026-10-31"));
    Ok(())
}

#[tokio::test]
async fn prefilter_and_llm_exclusions_are_skipped_and_batches_never_exceed_eight() -> Result<()> {
    let mock = MockLlm::new(vec![output(vec![]), output(vec![])]);
    let pipeline = Pipeline::new(mock.clone())?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let mut messages = vec![message("A", "Alice", "Good morning everyone")];
    messages.extend((0..9).map(|_| message("A", "Bob", "Trainer available for training; hire me")));
    let report = pipeline
        .ingest(SourceBatch::Messages(messages), &mut store, today())
        .await?;
    assert_eq!(report.skipped, 10);
    let requests = mock.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.messages.len())
            .collect::<Vec<_>>(),
        vec![8, 1]
    );
    assert_eq!(store.messages_by_status("skipped")?.len(), 10);
    Ok(())
}

#[tokio::test]
async fn screenshot_bypasses_prefilter_and_is_retained_for_review() -> Result<()> {
    let mock = MockLlm::new(vec!["invalid".into(), "invalid".into()]);
    let pipeline = Pipeline::new(mock.clone())?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let screenshot = Screenshot {
        message: message("Screenshot", "Alice", ""),
        media_type: "image/png".into(),
        bytes: vec![137, 80, 78, 71],
    };
    let report = pipeline
        .ingest(
            SourceBatch::Screenshot(screenshot.clone()),
            &mut store,
            today(),
        )
        .await?;
    assert_eq!(report.needs_review, 1);
    let review = store.messages_by_status("needs_review")?;
    assert_eq!(
        store.screenshot(review[0].id)?.unwrap().bytes,
        screenshot.bytes
    );
    assert!(mock.requests.lock().unwrap()[0].screenshot.is_some());
    Ok(())
}

#[tokio::test]
async fn vision_transcription_is_stored_as_the_original_sighting_text() -> Result<()> {
    let raw = "Training requirement AngularJS Mysuru 2-7 Nov";
    let response = serde_json::to_string(&ExtractionOutput {
        requirements: vec![ExtractionItem {
            message_index: 0,
            requirement: requirement("Mysuru"),
            raw_text: Some(raw.into()),
        }],
    })?;
    let pipeline = Pipeline::new(MockLlm::new(vec![response]))?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let report = pipeline
        .ingest(
            SourceBatch::Screenshot(Screenshot {
                message: message("Screenshot", "Alice", ""),
                media_type: "image/png".into(),
                bytes: vec![1],
            }),
            &mut store,
            today(),
        )
        .await?;
    assert_eq!(report.new, 1);
    assert_eq!(store.clusters()?[0].primary.message.raw_text, raw);
    Ok(())
}

#[test]
fn strict_schema_rejects_bad_dates_phones_unknown_keys_and_invalid_indexes() -> Result<()> {
    let request = ExtractionRequest {
        today: today(),
        messages: vec![message("A", "Alice", "Training")],
        screenshot: None,
        correction: false,
    };
    let good: serde_json::Value = serde_json::from_str(&output(vec![(0, requirement("Mysuru"))]))?;
    for (field, value) in [
        ("start_date", json!("2026-02-30")),
        ("end_date", json!("2026-11-01")),
        ("start_date", json!("2026-1-02")),
        ("vendor_phone", json!("+91 9876543210")),
        ("duration_days", json!(0)),
        ("mode", json!("remote")),
        ("surprise", json!(true)),
    ] {
        let mut bad = good.clone();
        bad["requirements"][0]["requirement"][field] = value;
        assert!(
            decode_output(&bad.to_string(), &request).is_err(),
            "accepted {field}"
        );
    }
    let mut bad = good;
    bad["requirements"][0]["message_index"] = json!(8);
    assert!(decode_output(&bad.to_string(), &request).is_err());
    bad["requirements"][0]["message_index"] = json!(0);
    bad["requirements"][0]["requirement"]
        .as_object_mut()
        .unwrap()
        .remove("rate");
    assert!(decode_output(&bad.to_string(), &request).is_err());
    Ok(())
}

#[test]
fn manual_merge_split_preserve_sources_and_recount_duplicates() -> Result<()> {
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let mut records = Vec::new();
    let messages = vec![
        message("A", "Alice", "Training AngularJS Mysuru"),
        message("B", "Bob", "Training AngularJS Mysuru"),
    ];
    let ids = store.record_messages(&messages)?;
    for (id, message) in ids.iter().zip(&messages) {
        records.push((*id, message.clone(), requirement("Mysuru")));
    }
    assert_eq!(
        store.save_batch(&ids, &records)?,
        vec![DuplicateKind::New, DuplicateKind::Exact]
    );
    let original = store.clusters()?.remove(0);
    let split = store.split_cluster(original.id, &[original.sources[1].id])?;
    assert_eq!(store.clusters()?.len(), 2);
    store.merge_clusters(original.id, split)?;
    let merged = store.clusters()?.remove(0);
    assert_eq!((merged.seen, merged.groups, merged.exact), (2, 2, 1));
    assert!(store.split_cluster(merged.id, &[]).is_err());
    assert!(store.split_cluster(merged.id, &[999]).is_err());
    assert!(store.merge_clusters(merged.id, 999).is_err());
    assert_eq!(store.clusters()?.len(), 1);
    Ok(())
}

#[test]
fn failed_batch_rolls_back_and_reprocessing_does_not_duplicate_sightings() -> Result<()> {
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let message = message("A", "Alice", "Training AngularJS");
    let ids = store.record_messages(std::slice::from_ref(&message))?;
    let mut bad = requirement("Mysuru");
    bad.duration_days = Some(0);
    assert!(
        store
            .save_batch(
                &ids,
                &[
                    (ids[0], message.clone(), requirement("Mysuru")),
                    (ids[0], message.clone(), bad)
                ]
            )
            .is_err()
    );
    assert!(store.clusters()?.is_empty());
    assert_eq!(store.messages_by_status("pending")?.len(), 1);
    let records = vec![(ids[0], message, requirement("Mysuru"))];
    store.save_batch(&ids, &records)?;
    assert!(store.save_batch(&ids, &records).is_err());
    assert_eq!(store.clusters()?[0].seen, 1);
    Ok(())
}

struct OfflineLlm;

#[async_trait]
impl LlmClient for OfflineLlm {
    async fn extract(&self, _: &ExtractionRequest) -> Result<String> {
        Err(Error::ApiStatus(503))
    }
}

#[tokio::test]
async fn unavailable_llm_leaves_input_pending_for_recovery() -> Result<()> {
    let pipeline = Pipeline::new(OfflineLlm)?;
    let mut store = Store::in_memory(DedupThresholds::default())?;
    assert!(
        pipeline
            .ingest(
                SourceBatch::Messages(vec![message("A", "Alice", "Training AngularJS")]),
                &mut store,
                today()
            )
            .await
            .is_err()
    );
    assert_eq!(store.messages_by_status("pending")?.len(), 1);
    assert!(store.clusters()?.is_empty());
    Ok(())
}

#[test]
fn sqlite_data_survives_close_and_reopen() -> Result<()> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("trainlead-{}-{unique}.db", std::process::id()));
    {
        let mut store = Store::open(&path, DedupThresholds::default())?;
        let message = message("Vendors", "Alice", "Training AngularJS Mysuru");
        let ids = store.record_messages(std::slice::from_ref(&message))?;
        store.save_batch(&ids, &[(ids[0], message, requirement("Mysuru"))])?;
    }
    {
        let store = Store::open(&path, DedupThresholds::default())?;
        assert_eq!(store.clusters()?[0].primary.message.source_group, "Vendors");
        assert_eq!(store.messages_by_status("processed")?.len(), 1);
    }
    std::fs::remove_file(path)?;
    Ok(())
}
