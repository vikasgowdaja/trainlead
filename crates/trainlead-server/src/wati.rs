use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, Url, header::{HeaderMap, HeaderValue, AUTHORIZATION}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use trainlead_core::{models::Message, source::{MessageSource, SourceBatch}};

#[derive(Debug, thiserror::Error)]
pub enum WatiError {
    #[error("WATI configuration is missing or invalid: {0}")]
    Configuration(&'static str),
    #[error("WATI request failed with HTTP {0}")]
    Status(u16),
    #[error("WATI could not be reached or returned an unreadable response")]
    Transport,
    #[error("WATI payload is invalid: {0}")]
    Payload(&'static str),
}

pub struct WatiClient { client: Client, base: Url }

impl WatiClient {
    pub fn from_env() -> Result<Self, WatiError> {
        let base = std::env::var("WATI_API_BASE_URL")
            .map_err(|_| WatiError::Configuration("WATI_API_BASE_URL"))?;
        let key = std::env::var("WATI_API_TOKEN")
            .map_err(|_| WatiError::Configuration("WATI_API_TOKEN"))?;
        Self::new(&base, &key)
    }

    pub fn new(base: &str, token: &str) -> Result<Self, WatiError> {
        let base = normalize_base(base)?;
        if token.trim().is_empty() { return Err(WatiError::Configuration("WATI_API_TOKEN")); }
        let mut value = HeaderValue::from_str(&format!("Bearer {}", token.trim()))
            .map_err(|_| WatiError::Configuration("invalid token characters"))?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        let client = Client::builder().default_headers(headers).timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| WatiError::Transport)?;
        Ok(Self { client, base })
    }

    pub async fn check(&self) -> Result<(), WatiError> {
        self.get("conversations", &[("page_number", "1".into()), ("page_size", "1".into()), ("channel_type", "whatsapp".into())]).await?;
        Ok(())
    }

    pub async fn conversations(&self, channel: &str) -> Result<Vec<Conversation>, WatiError> {
        if !["whatsapp", "whatsapp_group"].contains(&channel) { return Err(WatiError::Payload("invalid channel filter")); }
        let response = self.get("conversations", &[("page_number", "1".into()), ("page_size", "100".into()), ("channel_type", channel.into())]).await?;
        let items = response.get("conversation_list").ok_or(WatiError::Payload("conversation_list missing"))?;
        serde_json::from_value(items.clone()).map_err(|_| WatiError::Payload("invalid conversation list"))
    }

    pub async fn messages(&self, conversation: &Conversation) -> Result<Vec<Delivery>, WatiError> {
        if conversation.id.is_empty() || !conversation.id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_') {
            return Err(WatiError::Payload("invalid conversation ID"));
        }
        let response = self.get(&format!("conversations/{}/messages", conversation.id), &[("page_number", "1".into()), ("page_size", "100".into())]).await?;
        let items = response.get("message_list").ok_or(WatiError::Payload("message_list missing"))?;
        let messages: Vec<WatiMessage> = if items.is_null() { Vec::new() } else {
            serde_json::from_value(items.clone()).map_err(|_| WatiError::Payload("invalid message history"))?
        };
        let channel_id = response.get("channel_id").and_then(Value::as_str).map(str::to_string);
        let mut deliveries = Vec::new();
        for mut message in messages {
            message.conversation_id = Some(conversation.id.clone());
            if message.channel_id.is_none() { message.channel_id = channel_id.clone(); }
            if let Some(mut delivery) = message.delivery()? {
                if conversation.channel_type == "whatsapp_group" {
                    delivery.message.source_group = format!("WATI group / {}", conversation.contact_name);
                }
                deliveries.push(delivery);
            }
        }
        Ok(deliveries)
    }

    pub async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, WatiError> {
        let url = self.base.join(path).map_err(|_| WatiError::Configuration("invalid API path"))?;
        if url.host_str() != self.base.host_str() || !url.path().starts_with("/api/ext/v3/") {
            return Err(WatiError::Configuration("API path outside WATI V3"));
        }
        for attempt in 0..3 {
            let response = self.client.get(url.clone()).query(query).send().await.map_err(|_| WatiError::Transport)?;
            let status = response.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < 2 {
                let retry_after = response.headers().get("retry-after").and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok());
                tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1 << attempt).clamp(1, 30))).await;
                continue;
            }
            if !status.is_success() { return Err(WatiError::Status(status.as_u16())); }
            return response.json().await.map_err(|_| WatiError::Transport);
        }
        Err(WatiError::Transport)
    }
}

fn normalize_base(input: &str) -> Result<Url, WatiError> {
    let mut url = Url::parse(input.trim()).map_err(|_| WatiError::Configuration("invalid base URL"))?;
    if url.scheme() != "https" || !url.host_str().is_some_and(|host| host.ends_with(".wati.io"))
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
        || !["/", "/api/ext/v3", "/api/ext/v3/"].contains(&url.path()) || url.port().is_some_and(|port| port != 443) {
        return Err(WatiError::Configuration("use the HTTPS WATI host or its /api/ext/v3 base URL"));
    }
    url.set_path("/api/ext/v3/");
    Ok(url)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatiMessage {
    pub id: Option<String>,
    #[serde(alias = "whatsapp_message_id")]
    pub whatsapp_message_id: Option<String>,
    #[serde(alias = "conversation_id")]
    pub conversation_id: Option<String>,
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    pub timestamp: Option<String>,
    pub created: Option<String>,
    pub owner: bool,
    #[serde(alias = "event_type")]
    pub event_type: Option<String>,
    #[serde(alias = "sender_name")]
    pub sender_name: Option<String>,
    #[serde(alias = "wa_id")]
    pub wa_id: Option<String>,
    #[serde(alias = "channel_id")]
    pub channel_id: Option<String>,
    #[serde(alias = "channel_phone_number")]
    pub channel_phone_number: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Conversation { pub id: String, pub contact_name: String, pub channel_type: String }

#[derive(Clone, Serialize, Deserialize)]
pub struct Delivery { pub id: String, pub message: Message }

impl WatiMessage {
    pub fn delivery(self) -> Result<Option<Delivery>, WatiError> {
        if self.owner || self.event_type.as_deref().is_some_and(|event| !["message", "message_bsuid", "newContactMessageReceived", "newContactMessageReceived_bsuid"].contains(&event)) {
            return Ok(None);
        }
        if self.message_type.as_deref() != Some("text") { return Ok(None); }
        let text = self.text.filter(|text| !text.trim().is_empty()).ok_or(WatiError::Payload("missing message text"))?;
        if text.len() > 100_000 { return Err(WatiError::Payload("message too large")); }
        let id = self.whatsapp_message_id.or(self.id).filter(|id| !id.is_empty() && id.len() < 500)
            .ok_or(WatiError::Payload("missing message identifier"))?;
        let timestamp = self.created.as_deref().and_then(|created| DateTime::parse_from_rfc3339(created).ok())
            .map(|date| date.with_timezone(&Utc)).or_else(|| {
                let timestamp = self.timestamp.as_deref()?;
                DateTime::parse_from_rfc3339(timestamp).ok().map(|date| date.with_timezone(&Utc))
                    .or_else(|| timestamp.parse::<i64>().ok().and_then(|seconds| DateTime::from_timestamp(seconds, 0)))
            })
            .ok_or(WatiError::Payload("invalid message timestamp"))?;
        let channel = self.channel_id.or(self.channel_phone_number).unwrap_or_else(|| "default".into());
        let conversation = self.conversation_id.unwrap_or_else(|| "inbox".into());
        if channel.len() > 200 || conversation.len() > 200 { return Err(WatiError::Payload("invalid channel/conversation identifier")); }
        let sender = match (self.sender_name, self.wa_id) {
            (Some(name), Some(number)) => Some(format!("{name} ({number})")),
            (name, number) => name.or(number),
        };
        Ok(Some(Delivery {id:format!("wati:{channel}:{id}"), message:Message {
            source_group:format!("WATI inbox / {conversation}"), sender,
            timestamp:timestamp.to_rfc3339(), raw_text:text,
        }}))
    }
}

pub struct WatiSource { pub messages: Vec<Message> }

#[async_trait]
impl MessageSource for WatiSource {
    fn name(&self) -> &str { "wati" }
    async fn read(&mut self) -> trainlead_core::Result<SourceBatch> {
        Ok(SourceBatch::Messages(std::mem::take(&mut self.messages)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        serde_json::json!({"id":"record-1","whatsappMessageId":"wamid.1","conversationId":"thread-1",
            "text":"Training AngularJS Mysuru","type":"text","timestamp":"1793491200",
            "owner":false,"eventType":"message","senderName":"Vendor","waId":"919876543210",
            "channelPhoneNumber":"919111111111"})
    }

    #[test]
    fn maps_inbound_text_without_inventing_a_group() -> Result<(), Box<dyn std::error::Error>> {
        let message: WatiMessage = serde_json::from_value(payload())?;
        let delivery = message.delivery()?.ok_or("missing delivery")?;
        assert_eq!(delivery.id, "wati:919111111111:wamid.1");
        assert_eq!(delivery.message.source_group, "WATI inbox / thread-1");
        assert_eq!(delivery.message.raw_text, "Training AngularJS Mysuru");
        assert!(delivery.message.timestamp.ends_with("+00:00"));
        Ok(())
    }

    #[test]
    fn ignores_outgoing_status_and_media_events() -> Result<(), Box<dyn std::error::Error>> {
        for (field, value) in [("owner", Value::Bool(true)), ("eventType", Value::String("sessionMessageSent_v2".into())), ("type", Value::String("image".into()))] {
            let mut input = payload(); input[field] = value;
            assert!(serde_json::from_value::<WatiMessage>(input)?.delivery()?.is_none());
        }
        Ok(())
    }

    #[test]
    fn handles_snake_case_history_and_iso_timestamps() -> Result<(), Box<dyn std::error::Error>> {
        let input = serde_json::json!({"id":"event-1","conversation_id":"thread-1","channel_id":"channel-1", "event_type":"message",
            "text":"Training AngularJS", "type":"text","timestamp":"2026-11-01T09:00:00Z","owner":false});
        let delivery = serde_json::from_value::<WatiMessage>(input)?.delivery()?.ok_or("missing delivery")?;
        assert_eq!(delivery.id, "wati:channel-1:event-1");
        assert_eq!(delivery.message.timestamp, "2026-11-01T09:00:00+00:00");
        Ok(())
    }

    #[test]
    fn rejects_unsafe_hosts_and_normalizes_base_path() -> Result<(), WatiError> {
        assert_eq!(normalize_base("https://live-mt-server.wati.io/api/ext/v3")?.as_str(), "https://live-mt-server.wati.io/api/ext/v3/");
        for base in ["http://live-mt-server.wati.io", "https://evil.example", "https://live-mt-server.wati.io@evil.example", "https://live-mt-server.wati.io/api/v1", "https://live-mt-server.wati.io/?key=secret"] {
            assert!(normalize_base(base).is_err());
        }
        Ok(())
    }
}