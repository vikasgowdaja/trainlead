use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub source_group: String,
    pub sender: Option<String>,
    pub timestamp: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provision {
    Provided,
    NotProvided,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Offline,
    Online,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub tech_stack: Vec<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub duration_days: Option<u32>,
    pub college: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub mode: Option<Mode>,
    pub audience: Option<String>,
    pub travel: Provision,
    pub food: Provision,
    pub accommodation: Provision,
    pub logistics_note: Option<String>,
    pub rate: Option<String>,
    pub payment_date: Option<NaiveDate>,
    pub vendor_name: Option<String>,
    pub vendor_company: Option<String>,
    pub vendor_phone: Option<String>,
    pub summary: String,
}

impl Requirement {
    pub fn normalize_and_validate(&mut self) -> Result<()> {
        if self.summary.trim().is_empty() || self.summary.contains(['\r', '\n']) {
            return Err(Error::Validation("summary must be one nonempty line".into()));
        }
        if let (Some(start), Some(end)) = (self.start_date, self.end_date)
            && end < start
        {
            return Err(Error::Validation("end_date precedes start_date".into()));
        }
        if self.duration_days == Some(0) {
            return Err(Error::Validation("duration_days must be positive".into()));
        }
        if let Some(phone) = &self.vendor_phone
            && (!(7..=15).contains(&phone.len()) || !phone.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(Error::Validation("phone must contain 7-15 digits including country code".into()));
        }
        for value in [
            &mut self.college, &mut self.city, &mut self.state, &mut self.audience,
            &mut self.logistics_note, &mut self.rate, &mut self.vendor_name,
            &mut self.vendor_company,
        ] {
            *value = value.take().map(|text| text.trim().to_string()).filter(|text| !text.is_empty());
        }
        self.summary = self.summary.trim().to_string();
        self.tech_stack = self.tech_stack.iter().map(|tech| normalize_tech(tech))
            .filter(|tech| !tech.is_empty()).collect();
        self.tech_stack.sort_by_key(|tech| tech.to_lowercase());
        self.tech_stack.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        Ok(())
    }

    pub fn completeness(&self) -> usize {
        let strings = [
            &self.college, &self.city, &self.state, &self.audience, &self.logistics_note,
            &self.rate, &self.vendor_name, &self.vendor_company, &self.vendor_phone,
        ];
        strings.iter().filter(|value| value.as_ref().is_some_and(|text| !text.is_empty())).count()
            + usize::from(!self.tech_stack.is_empty())
            + usize::from(self.start_date.is_some()) + usize::from(self.end_date.is_some())
            + usize::from(self.duration_days.is_some()) + usize::from(self.payment_date.is_some())
            + usize::from(self.mode.is_some())
            + [self.travel, self.food, self.accommodation].iter()
                .filter(|value| **value != Provision::Unknown).count()
            + usize::from(!self.summary.is_empty())
    }
}

pub fn normalize_tech(value: &str) -> String {
    let compact: String = value.chars().filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase).collect();
    match compact.as_str() {
        "angularjs" => "AngularJS".into(),
        "angular" => "Angular".into(),
        "react" | "reactjs" => "React".into(),
        "node" | "nodejs" => "Node.js".into(),
        "javascript" => "JavaScript".into(),
        "typescript" => "TypeScript".into(),
        "python" => "Python".into(),
        "java" => "Java".into(),
        "dotnet" | "net" => ".NET".into(),
        _ => value.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateKind { New, Exact, Similar }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sighting {
    pub id: i64,
    pub cluster_id: i64,
    pub kind: DuplicateKind,
    #[serde(flatten)]
    pub message: Message,
    pub requirement: Requirement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: i64,
    pub primary: Sighting,
    pub seen: usize,
    pub groups: usize,
    pub exact: usize,
    pub similar: usize,
    pub sources: Vec<Sighting>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestReport {
    pub found: usize,
    pub new: usize,
    pub exact: usize,
    pub similar: usize,
    pub skipped: usize,
    pub needs_review: usize,
}