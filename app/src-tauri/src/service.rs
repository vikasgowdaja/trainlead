use std::path::{Path, PathBuf};

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

pub async fn capture<L: LlmClient>(
    path: &Path,
    text: &str,
    group: &str,
    export: bool,
    llm: L,
) -> Result<IngestReport> {
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
    // Pasted text copied from WhatsApp carries a header per message; split it like an export.
    let copied = if export { Vec::new() } else { parse_export(text, group).unwrap_or_default() };
    let messages = if export {
        parse_export(text, group)?
    } else if !copied.is_empty() {
        copied
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
    let result = Pipeline::new(llm)?
        .ingest(
            SourceBatch::Messages(messages),
            &mut store,
            Local::now().date_naive(),
        )
        .await;
    park_on_failure(&mut store, result)
}

/// A WhatsApp message captured from an Android notification.
#[derive(Debug, serde::Deserialize)]
pub struct NotificationMessage {
    pub group: String,
    pub sender: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub timestamp: i64,
    pub text: String,
}

pub const MAX_NOTIFICATION_BATCH: usize = 200;

pub async fn capture_notifications<L: LlmClient>(
    path: &Path,
    notifications: Vec<NotificationMessage>,
    llm: L,
) -> Result<IngestReport> {
    if notifications.len() > MAX_NOTIFICATION_BATCH {
        return Err(Error::Validation(format!(
            "import at most {MAX_NOTIFICATION_BATCH} notifications at a time"
        )));
    }
    let messages: Vec<Message> = notifications
        .into_iter()
        .filter_map(|notification| {
            let text = notification.text.trim();
            let group = notification.group.trim();
            if text.is_empty() || text.len() > 64 * 1024 || group.is_empty() || group.len() > 200 {
                return None;
            }
            let timestamp = chrono::DateTime::from_timestamp_millis(notification.timestamp)
                .map(|time| time.with_timezone(&Local).to_rfc3339())
                .unwrap_or_else(|| Local::now().to_rfc3339());
            Some(Message {
                source_group: group.into(),
                sender: notification
                    .sender
                    .map(|sender| sender.trim().chars().take(200).collect::<String>())
                    .filter(|sender| !sender.is_empty()),
                timestamp,
                raw_text: text.into(),
            })
        })
        .collect();
    if messages.is_empty() {
        return Ok(IngestReport::default());
    }
    let mut store = open(path)?;
    let result = Pipeline::new(llm)?
        .ingest(
            SourceBatch::Messages(messages),
            &mut store,
            Local::now().date_naive(),
        )
        .await;
    park_on_failure(&mut store, result)
}

/// Re-extracts review-inbox messages (all of them when `ids` is None) without duplicating them.
pub async fn retry<L: LlmClient>(path: &Path, ids: Option<Vec<i64>>, llm: L) -> Result<IngestReport> {
    let mut store = open(path)?;
    let ids = match ids {
        Some(ids) => ids,
        None => store
            .messages_by_status("needs_review")?
            .into_iter()
            .map(|message| message.id)
            .collect(),
    };
    let result = Pipeline::new(llm)?
        .retry(&ids, &mut store, Local::now().date_naive())
        .await;
    park_on_failure(&mut store, result)
}

fn park_on_failure(store: &mut Store, result: Result<IngestReport>) -> Result<IngestReport> {
    match result {
        Ok(report) => Ok(report),
        Err(error) => {
            // Keep messages the pipeline left pending so a Groq outage never loses them.
            let pending: Vec<i64> = store
                .messages_by_status("pending")?
                .into_iter()
                .map(|message| message.id)
                .collect();
            store.mark_messages(
                &pending,
                "needs_review",
                Some(&format!("Groq extraction failed: {error}")),
            )?;
            Ok(IngestReport {
                needs_review: pending.len(),
                ..Default::default()
            })
        }
    }
}

fn key_path(directory: &Path) -> PathBuf {
    directory.join("groq_api_key")
}

pub fn saved_key(directory: &Path) -> Option<String> {
    std::fs::read_to_string(key_path(directory))
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// Stored in the app's private data directory (app sandbox on Android), not in SQLite.
pub fn save_key(directory: &Path, key: &str) -> Result<()> {
    let temporary = directory.join("groq_api_key.tmp");
    std::fs::write(&temporary, key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(temporary, key_path(directory))?;
    Ok(())
}

pub fn remove_key(directory: &Path) -> Result<()> {
    match std::fs::remove_file(key_path(directory)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
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
    fn capture_without_groq_is_extracted_by_on_device_rules() -> Result<()> {
        let path = database();
        let rules = std::sync::Arc::new(trainlead_core::rules::RuleExtractor::new()?);
        let extractor = std::sync::Arc::new(trainlead_core::rules::FallbackExtractor::<DemoLlm>::new(None, rules));
        let report = tauri::async_runtime::block_on(capture(
            &path,
            "Training requirement: AngularJS, Mysuru, 2-7 Nov, TFA provided",
            "Vendors",
            false,
            extractor.clone(),
        ))?;
        assert_eq!((report.found, report.needs_review), (1, 0));
        assert_eq!(extractor.engine(), "on-device rules");
        {
            let store = open(&path)?;
            assert!(store.messages_by_status("needs_review")?.is_empty());
            assert_eq!(store.clusters()?[0].primary.requirement.city.as_deref(), Some("Mysuru"));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn notification_messages_keep_group_sender_and_time_and_skip_chatter() -> Result<()> {
        let path = database();
        let rules = std::sync::Arc::new(trainlead_core::rules::RuleExtractor::new()?);
        let extractor = std::sync::Arc::new(trainlead_core::rules::FallbackExtractor::<DemoLlm>::new(None, rules));
        let note = |group: &str, sender: Option<&str>, text: &str| NotificationMessage {
            group: group.into(),
            sender: sender.map(Into::into),
            timestamp: 1_793_500_000_000,
            text: text.into(),
        };
        let report = tauri::async_runtime::block_on(capture_notifications(
            &path,
            vec![
                note("Trainer Network", Some("Asha"), "Training requirement: AngularJS, Mysuru, 2-7 Nov, TFA provided"),
                note("Trainer Network", Some("Ravi"), "Good morning all"),
                note("  ", None, "ignored: no group"),
            ],
            extractor,
        ))?;
        assert_eq!(report.found, 1);
        {
            let clusters = open(&path)?.clusters()?;
            let source = &clusters[0].sources[0].message;
            assert_eq!(source.source_group, "Trainer Network");
            assert_eq!(source.sender.as_deref(), Some("Asha"));
            assert!(source.timestamp.starts_with("2026-11-0"));
        }
        let too_many = (0..=MAX_NOTIFICATION_BATCH).map(|_| note("G", None, "x")).collect();
        let rules = std::sync::Arc::new(trainlead_core::rules::RuleExtractor::new()?);
        let extractor = std::sync::Arc::new(trainlead_core::rules::FallbackExtractor::<DemoLlm>::new(None, rules));
        assert!(tauri::async_runtime::block_on(capture_notifications(&path, too_many, extractor)).is_err());
        std::fs::remove_file(path)?;
        Ok(())
    }

    struct OfflineLlm;

    #[async_trait]
    impl LlmClient for OfflineLlm {
        async fn extract(&self, _: &ExtractionRequest) -> Result<String> {
            Err(Error::ApiStatus(503))
        }
    }

    #[test]
    fn failed_groq_call_moves_messages_to_review_instead_of_losing_them() -> Result<()> {
        let path = database();
        let report = tauri::async_runtime::block_on(capture(
            &path,
            "Training requirement AngularJS Mysuru",
            "Vendors",
            false,
            OfflineLlm,
        ))?;
        assert_eq!((report.found, report.needs_review), (0, 1));
        {
            let store = open(&path)?;
            assert!(store.messages_by_status("pending")?.is_empty());
            let review = store.messages_by_status("needs_review")?;
            assert!(
                review[0]
                    .review_reason
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("Groq extraction failed"))
            );
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn retry_extracts_inbox_messages_and_failures_return_to_the_inbox() -> Result<()> {
        let path = database();
        tauri::async_runtime::block_on(capture(&path, SAMPLE, "Vendors", false, OfflineLlm))?;
        let failed = tauri::async_runtime::block_on(retry(&path, None, OfflineLlm))?;
        assert_eq!((failed.found, failed.needs_review), (0, 1));
        let report = tauri::async_runtime::block_on(retry(&path, None, DemoLlm))?;
        assert_eq!((report.found, report.new, report.needs_review), (1, 1, 0));
        {
            let store = open(&path)?;
            assert!(store.messages_by_status("needs_review")?.is_empty());
            assert_eq!(store.messages_by_status("processed")?.len(), 1);
            assert_eq!(store.clusters()?.len(), 1);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn api_key_is_saved_trimmed_and_removable() -> Result<()> {
        let directory = database().with_extension("keys");
        std::fs::create_dir_all(&directory)?;
        assert_eq!(saved_key(&directory), None);
        save_key(&directory, "gsk_example_key_for_tests")?;
        assert_eq!(saved_key(&directory).as_deref(), Some("gsk_example_key_for_tests"));
        remove_key(&directory)?;
        remove_key(&directory)?;
        assert_eq!(saved_key(&directory), None);
        std::fs::remove_dir_all(directory)?;
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
