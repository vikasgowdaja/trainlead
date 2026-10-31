use std::path::Path;

use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate};
use trainlead_core::{
    Error, Result,
    dedup::DedupThresholds,
    extraction::{ExtractionItem, ExtractionOutput, ExtractionRequest, LlmClient},
    models::{IngestReport, Message, Mode, Provision, Requirement},
    parser::parse_export,
    pipeline::Pipeline,
    source::SourceBatch,
    store::Store,
};

const SAMPLE: &str = "Training requirement: AngularJS, Mysuru Engineering College, Mysuru, 2-7 Nov, 6th sem engg, TFA provided, payment 25 Nov";
const REPOST: &str = "College workshop: Angular JS instructor needed in Mysuru Engineering College, Mysuru, November 2 through 7. Sixth semester engineering. Travel, food and stay covered; payment November 25.";
const OTHER: &str = "Training requirement: AngularJS, Bengaluru, 12-17 Nov, TFA provided";

pub fn open(path: &Path) -> Result<Store> {
    Store::open(path, DedupThresholds::default())
}

pub fn capture(path: &Path, text: &str, group: &str, export: bool) -> Result<IngestReport> {
    if text.trim().is_empty() || text.len() > 2 * 1024 * 1024 {
        return Err(Error::Validation(
            "input must contain text and be at most 2 MiB".into(),
        ));
    }
    let group = group.trim();
    if group.is_empty() || group.len() > 200 {
        return Err(Error::Validation(
            "source group must contain 1-200 bytes".into(),
        ));
    }
    let messages = if export {
        parse_export(text, group)?
    } else {
        vec![Message {
            source_group: group.into(),
            sender: None,
            timestamp: Local::now().to_rfc3339(),
            raw_text: text.to_string(),
        }]
    };
    if messages.is_empty() {
        return Err(Error::Validation("no messages found in export".into()));
    }
    let mut store = open(path)?;
    let ids = store.record_messages(&messages)?;
    store.mark_messages(
        &ids,
        "needs_review",
        Some("Live extraction is not configured in this emulator shell"),
    )?;
    Ok(IngestReport {
        needs_review: ids.len(),
        ..Default::default()
    })
}

pub async fn load_demo(path: &Path) -> Result<IngestReport> {
    let mut store = open(path)?;
    let pipeline = Pipeline::new(DemoLlm)?;
    let timestamp = Local::now().to_rfc3339();
    let messages = [
        ("Demo: Training vendors", "Asha", SAMPLE),
        ("Demo: College connect", "Meera", SAMPLE),
        ("Demo: Trainer network", "Ravi", REPOST),
        ("Demo: Training vendors", "Asha", OTHER),
    ]
    .into_iter()
    .map(|(group, sender, raw)| Message {
        source_group: group.into(),
        sender: Some(sender.into()),
        timestamp: timestamp.clone(),
        raw_text: raw.into(),
    })
    .collect();
    pipeline
        .ingest(
            SourceBatch::Messages(messages),
            &mut store,
            Local::now().date_naive(),
        )
        .await
}

struct DemoLlm;

#[async_trait]
impl LlmClient for DemoLlm {
    async fn extract(&self, request: &ExtractionRequest) -> Result<String> {
        let mut requirements = Vec::new();
        for (message_index, message) in request.messages.iter().enumerate() {
            let mut requirement = fixture(request.today.year())?;
            match message.raw_text.as_str() {
                SAMPLE => {}
                REPOST => {
                    requirement.vendor_name = Some("Ravi".into());
                    requirement.vendor_company = Some("Demo vendor B".into());
                }
                OTHER => {
                    requirement.city = Some("Bengaluru".into());
                    requirement.college = None;
                    requirement.start_date = Some(november(request.today.year(), 12)?);
                    requirement.end_date = Some(november(request.today.year(), 17)?);
                    requirement.summary = "AngularJS training in Bengaluru".into();
                }
                _ => {
                    return Err(Error::Validation(
                        "demo client only accepts fixed fixtures".into(),
                    ));
                }
            }
            requirements.push(ExtractionItem {
                message_index,
                requirement,
                raw_text: None,
            });
        }
        Ok(serde_json::to_string(&ExtractionOutput { requirements })?)
    }
}

fn november(year: i32, day: u32) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(year, 11, day)
        .ok_or_else(|| Error::Validation("invalid demo date".into()))
}

fn fixture(year: i32) -> Result<Requirement> {
    Ok(Requirement {
        tech_stack: vec!["Angular JS".into()],
        start_date: Some(november(year, 2)?),
        end_date: Some(november(year, 7)?),
        duration_days: Some(6),
        college: Some("Mysuru Engineering College".into()),
        city: Some("Mysuru".into()),
        state: Some("Karnataka".into()),
        mode: Some(Mode::Offline),
        audience: Some("6th sem Engg".into()),
        travel: Provision::Provided,
        food: Provision::Provided,
        accommodation: Provision::Provided,
        logistics_note: Some("TFA provided".into()),
        rate: None,
        payment_date: Some(november(year, 25)?),
        vendor_name: Some("Asha".into()),
        vendor_company: Some("Demo vendor A".into()),
        vendor_phone: None,
        summary: "AngularJS training in Mysuru".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "trainlead-shell-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ))
    }

    #[test]
    fn local_capture_is_durable_and_does_not_claim_extraction() -> Result<()> {
        let path = database();
        let report = capture(&path, "Training AngularJS", "Vendors", false)?;
        assert_eq!((report.found, report.needs_review), (0, 1));
        {
            let store = open(&path)?;
            assert_eq!(store.messages_by_status("needs_review")?.len(), 1);
            assert!(store.clusters()?.is_empty());
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn demo_uses_the_real_core_for_exact_and_similar_clusters() -> Result<()> {
        let path = database();
        let report = tauri::async_runtime::block_on(load_demo(&path))?;
        assert_eq!(
            (report.found, report.new, report.exact, report.similar),
            (4, 2, 1, 1)
        );
        {
            let store = open(&path)?;
            assert_eq!(store.clusters()?.len(), 2);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}
