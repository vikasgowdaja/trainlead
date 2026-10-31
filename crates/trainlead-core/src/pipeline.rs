use chrono::NaiveDate;
use regex::Regex;

use crate::{
    Error, Result,
    extraction::{ExtractionRequest, LlmClient, MAX_BATCH, decode_output},
    models::{DuplicateKind, IngestReport, Message},
    source::{MessageSource, Screenshot, SourceBatch},
    store::Store,
};

pub struct Pipeline<L> {
    llm: L,
    keywords: Regex,
}

impl<L: LlmClient> Pipeline<L> {
    pub fn new(llm: L) -> Result<Self> {
        let keywords = Regex::new(r"(?i)\b(train\w*|workshop\w*|fdp|batch\w*|dates?|college\w*|requirement\w*|bootcamp\w*|webinar\w*|session\w*)\b")
            .map_err(|error| Error::Validation(error.to_string()))?;
        Ok(Self { llm, keywords })
    }

    pub async fn ingest_source(
        &self,
        source: &mut dyn MessageSource,
        store: &mut Store,
        today: NaiveDate,
    ) -> Result<IngestReport> {
        self.ingest(source.read().await?, store, today).await
    }

    pub async fn ingest(
        &self,
        batch: SourceBatch,
        store: &mut Store,
        today: NaiveDate,
    ) -> Result<IngestReport> {
        let (messages, screenshot) = match batch {
            SourceBatch::Messages(messages) => (messages, None),
            SourceBatch::Screenshot(screenshot) => {
                screenshot.validate()?;
                (vec![screenshot.message.clone()], Some(screenshot))
            }
        };
        let ids = if let Some(screenshot) = &screenshot {
            vec![store.record_screenshot(screenshot)?]
        } else {
            store.record_messages(&messages)?
        };
        let recorded = ids.into_iter().zip(messages).collect();
        self.process(recorded, screenshot, store, today).await
    }

    /// Re-extracts messages from the review inbox in place, without recording duplicates.
    pub async fn retry(&self, ids: &[i64], store: &mut Store, today: NaiveDate) -> Result<IngestReport> {
        let requeued = store.requeue_review(ids)?;
        self.process(requeued, None, store, today).await
    }

    async fn process(
        &self,
        recorded: Vec<(i64, Message)>,
        screenshot: Option<Screenshot>,
        store: &mut Store,
        today: NaiveDate,
    ) -> Result<IngestReport> {
        let mut report = IngestReport::default();
        let mut candidates: Vec<(i64, Message)> = Vec::new();
        let mut skipped = Vec::new();
        for (id, message) in recorded {
            if screenshot.is_some() || self.keywords.is_match(&message.raw_text) {
                candidates.push((id, message));
            } else {
                skipped.push(id);
            }
        }
        store.mark_messages(&skipped, "skipped", None)?;
        report.skipped = skipped.len();
        for chunk in candidates.chunks(MAX_BATCH) {
            let batch_ids: Vec<i64> = chunk.iter().map(|(id, _)| *id).collect();
            let mut request = ExtractionRequest {
                today,
                messages: chunk.iter().map(|(_, message)| message.clone()).collect(),
                screenshot: screenshot.clone(),
                correction: false,
            };
            let mut valid = None;
            let mut failure = String::new();
            for attempt in 0..2 {
                request.correction = attempt == 1;
                let response = self.llm.extract(&request).await?;
                match decode_output(&response, &request) {
                    Ok(items) => {
                        valid = Some(items);
                        break;
                    }
                    Err(error) => {
                        failure = error.to_string();
                    }
                }
            }
            let Some(items) = valid else {
                store.mark_messages(&batch_ids, "needs_review", Some(&failure))?;
                report.needs_review += batch_ids.len();
                continue;
            };
            let mut records = Vec::with_capacity(items.len());
            for item in items {
                let (id, original) = &chunk[item.message_index];
                let mut message = original.clone();
                if let Some(raw_text) = item.raw_text {
                    message.raw_text = raw_text;
                }
                records.push((*id, message, item.requirement));
            }
            report.skipped += batch_ids
                .iter()
                .filter(|id| !records.iter().any(|(record_id, _, _)| record_id == *id))
                .count();
            let kinds = store.save_batch(&batch_ids, &records)?;
            report.found += kinds.len();
            for kind in kinds {
                match kind {
                    DuplicateKind::New => report.new += 1,
                    DuplicateKind::Exact => report.exact += 1,
                    DuplicateKind::Similar => report.similar += 1,
                }
            }
        }
        Ok(report)
    }
}
