use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Error, Result, models::Message, parser::parse_export};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    pub message: Message,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl Screenshot {
    pub fn validate(&self) -> Result<()> {
        if !["image/jpeg", "image/png", "image/gif", "image/webp"]
            .contains(&self.media_type.as_str())
        {
            return Err(Error::Validation(
                "unsupported screenshot media type".into(),
            ));
        }
        if self.bytes.is_empty() || self.bytes.len() > 5 * 1024 * 1024 {
            return Err(Error::Validation(
                "screenshot must be between 1 byte and 5 MiB".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum SourceBatch {
    Messages(Vec<Message>),
    Screenshot(Screenshot),
}

#[async_trait]
pub trait MessageSource: Send + Sync {
    fn name(&self) -> &str;
    async fn read(&mut self) -> Result<SourceBatch>;
}

pub struct PasteSource {
    pub messages: Vec<Message>,
}

#[async_trait]
impl MessageSource for PasteSource {
    fn name(&self) -> &str {
        "paste"
    }
    async fn read(&mut self) -> Result<SourceBatch> {
        Ok(SourceBatch::Messages(std::mem::take(&mut self.messages)))
    }
}

pub struct ExportSource {
    pub text: String,
    pub source_group: String,
}

#[async_trait]
impl MessageSource for ExportSource {
    fn name(&self) -> &str {
        "export"
    }
    async fn read(&mut self) -> Result<SourceBatch> {
        Ok(SourceBatch::Messages(parse_export(
            &self.text,
            &self.source_group,
        )?))
    }
}

pub struct ScreenshotSource {
    pub screenshot: Option<Screenshot>,
}

#[async_trait]
impl MessageSource for ScreenshotSource {
    fn name(&self) -> &str {
        "screenshot"
    }
    async fn read(&mut self) -> Result<SourceBatch> {
        let screenshot = self
            .screenshot
            .take()
            .ok_or_else(|| Error::Validation("screenshot source already consumed".into()))?;
        screenshot.validate()?;
        Ok(SourceBatch::Screenshot(screenshot))
    }
}
