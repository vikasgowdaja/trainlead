//! On-device, rule-based extraction: regular expressions plus skill and city vocabularies.
//! It needs no network or API key and emits the same JSON contract as the LLM extractor.
mod dates;
mod lexicon;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::NaiveDate;
use regex::Regex;

use crate::{
    Error, Result,
    extraction::{ExtractionItem, ExtractionOutput, ExtractionRequest, LlmClient, decode_output},
    models::{Mode, Provision, Requirement},
};
use dates::DateParser;

fn compile(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|error| Error::Validation(format!("invalid extraction rule: {error}")))
}

/// Matches whole terms, allowing symbols such as C++ and .NET that `\b` cannot bound.
fn term(alternatives: &str) -> Result<Regex> {
    compile(&format!(r"(?i)(?:^|[^a-z0-9])(?:{alternatives})(?:[^a-z0-9+#]|$)"))
}

const GENERIC_PREFIX: &[&str] = &[
    "training", "requirement", "urgent", "for", "at", "in", "the", "trainer", "needed", "workshop",
    "fdp", "hiring", "location", "venue", "client", "dear", "required", "hi", "hello",
];

pub struct RuleExtractor {
    tech: Vec<(Regex, &'static str, &'static [&'static str])>,
    cities: Vec<(Regex, &'static str, &'static str)>,
    dates: DateParser,
    header: [Regex; 2],
    availability: Regex,
    job: Regex,
    phone: Regex,
    rate: Regex,
    rate_label: Regex,
    college: Regex,
    contact: Regex,
    name_line: Regex,
    no_tfa: Regex,
    tfa: Regex,
    online: Regex,
    offline: Regex,
    hybrid: Regex,
    semester: Regex,
    study_year: Regex,
    degree: Regex,
    duration: Regex,
}

impl RuleExtractor {
    pub fn new() -> Result<Self> {
        Ok(Self {
            tech: lexicon::TECH.iter().map(|(pattern, name, redundant)| Ok((term(pattern)?, *name, *redundant))).collect::<Result<_>>()?,
            cities: lexicon::CITIES.iter().map(|(pattern, city, state)| Ok((term(pattern)?, *city, *state))).collect::<Result<_>>()?,
            dates: DateParser::new()?,
            header: [
                compile(r"(?m)^\s*\[[^\]\n]{4,40}\]\s*")?,
                compile(r"(?m)^\s*\d{1,2}[/.]\d{1,2}[/.]\d{2,4},?\s+\d{1,2}:\d{2}[^-\n]{0,12}-\s*")?,
            ],
            availability: compile(r"(?i)\b(?:i am|i'm|myself)\b[^\n]{0,60}\btrainer\b|\bavailable\s+for\s+(?:training|sessions?|batches|assignments|freelance)|\bopen\s+(?:to|for)\s+(?:new\s+)?(?:training\s+)?(?:assignments|opportunities|engagements)")?,
            job: compile(r"(?i)\b(?:salary|ctc|lpa|per annum|full[- ]time|permanent\s+(?:role|position)|job\s+(?:opening|opportunity|role|description)|notice\s+period|work\s+from\s+office|referrals?)\b|\b(?:hiring|looking\s+for)\b[^\n]{0,60}\b(?:developers?|engineers?|consultants?)\b")?,
            phone: compile(r"\+?\d[\d \-]{8,16}\d")?,
            rate: compile(r"(?i)(?:₹|\brs\.?|\binr)\s*\d[\d,]*(?:\.\d+)?\s*(?:k\b|/-)?(?:\s*(?:/|per|a)\s*(?:day|hour|hr|session|month)\b)?|\b\d[\d,]*(?:\.\d+)?\s*(?:k\b|/-)?\s*(?:/|per|a)\s*(?:day|hour|hr|session)\b|\b\d+(?:\.\d+)?\s*k\b(?:\s*(?:/|per)\s*(?:day|hour|hr|session)\b)?")?,
            rate_label: compile(r"(?i)\b(?:rate|budget|commercials?|remuneration|fees?|charges?|compensation|price)\b\s*[:\-–]?\s*([^\n]{1,40})")?,
            college: compile(r"(?:[A-Z][A-Za-z0-9_.&'’-]*\s+){1,6}(?i:college|university|institute|polytechnic|academy|vidyapeeth|vidyalaya)(?:\s+of\s+(?:[A-Z][A-Za-z0-9_&-]*\s*){1,4})?")?,
            contact: compile(r"(?:[Cc]ontact(?:\s+[Pp]erson)?|[Cc]all|[Rr]egards|[Ww]hats[Aa]pp)\s*[:\-–,]?\s*\n?\s*([A-Z][a-zA-Z.]+(?:[ \t]+[A-Z][a-zA-Z.]+){0,3})")?,
            name_line: compile(r"^[A-Z][a-zA-Z.]+(?:\s+[A-Z][a-zA-Z.]+){0,3}$")?,
            no_tfa: compile(r"(?i)\b(?:no|without|not)\s+(?:tfa|t\.f\.a)\b|\btfa\s*(?::\s*)?(?:not|no)\b")?,
            tfa: compile(r"(?i)\btfa\b|\bt\.f\.a\b")?,
            online: term(r"online|virtual|remote|zoom|google\s?meet|ms\s?teams|microsoft\s?teams|webex")?,
            offline: term(r"offline|on-?\s?site|in-?\s?person|classroom|on\s?campus|face\s?to\s?face|f2f")?,
            hybrid: term("hybrid")?,
            semester: compile(r"(?i)\b(\d)(?:st|nd|rd|th)?\s*sem(?:ester)?\b\.?\s*(b\.?\s?tech|b\.?e\b|engg|engineering|mca|mba|bca|b\.?sc|cse|ece|eee|it\b|mech|civil|diploma)?")?,
            study_year: compile(r"(?i)\b(1st|2nd|3rd|4th|first|second|third|fourth|final|pre-?final)\s+year\b\.?\s*(b\.?\s?tech|b\.?e\b|engg|engineering|mca|mba|bca|b\.?sc)?")?,
            degree: compile(r"(?i)\b(b\.?\s?tech|m\.?\s?tech|mca|mba|bca|bba|b\.?sc|m\.?sc|diploma)\b")?,
            duration: compile(r"(?i)\b(\d{1,2})\s*-?\s*days?\b")?,
        })
    }

    /// Structured requirements found in one message; empty when it is not a training requirement.
    pub fn requirements(&self, raw: &str, sender: Option<&str>, today: NaiveDate) -> Vec<Requirement> {
        let text = self.header.iter().fold(raw.to_string(), |text, header| header.replace_all(&text, "").into_owned());
        if self.availability.is_match(&text) { return Vec::new(); }
        let schedule = self.dates.schedule(&text, today);
        if self.job.is_match(&text) && schedule.start.is_none() { return Vec::new(); }
        let tech = self.tech(&text);
        let cities = self.cities(&text);
        if tech.is_empty() && schedule.start.is_none() && cities.is_empty() { return Vec::new(); }
        let college = self.college(&text);
        let (travel, food, accommodation, logistics_note) = self.logistics(&text);
        let duration_days = match (schedule.start, schedule.end) {
            (Some(start), Some(end)) => u32::try_from((end - start).num_days() + 1).ok(),
            _ => self.duration.captures(&text).and_then(|captures| captures[1].parse().ok()).filter(|days| (1..=90).contains(days)),
        };
        let places: Vec<Option<(&str, &str)>> = if cities.len() > 1 && college.is_none() {
            cities.iter().copied().map(Some).collect()
        } else {
            vec![cities.first().copied()]
        };
        let template = Requirement {
            tech_stack: tech.clone(), start_date: schedule.start, end_date: schedule.end, duration_days,
            college, city: None, state: None, mode: self.mode(&text), audience: self.audience(&text),
            travel, food, accommodation, logistics_note, rate: self.rate(&text), payment_date: schedule.payment,
            vendor_name: self.vendor_name(&text, sender), vendor_company: None,
            vendor_phone: self.phone(raw).or_else(|| sender.and_then(|sender| self.phone(sender))),
            summary: String::new(),
        };
        places.into_iter().filter_map(|place| {
            let mut requirement = template.clone();
            requirement.city = place.map(|(city, _)| city.to_string());
            requirement.state = place.map(|(_, state)| state.to_string());
            requirement.summary = summary(&tech, requirement.city.as_deref(), schedule.start, schedule.end);
            if requirement.clone().normalize_and_validate().is_err() { requirement.vendor_phone = None; }
            requirement.clone().normalize_and_validate().is_ok().then_some(requirement)
        }).collect()
    }

    fn tech(&self, text: &str) -> Vec<String> {
        let matched: Vec<_> = self.tech.iter().filter(|(regex, _, _)| regex.is_match(text)).collect();
        let redundant: Vec<&str> = matched.iter().flat_map(|(_, _, redundant)| redundant.iter().copied()).collect();
        matched.iter().map(|(_, name, _)| *name).filter(|name| !redundant.contains(name)).map(str::to_string).collect()
    }

    fn cities(&self, text: &str) -> Vec<(&'static str, &'static str)> {
        let mut found: Vec<(usize, &str, &str)> = self.cities.iter()
            .filter_map(|(regex, city, state)| regex.find(text).map(|found| (found.start(), *city, *state))).collect();
        found.sort_by_key(|(position, _, _)| *position);
        let mut cities: Vec<(&str, &str)> = Vec::new();
        for (_, city, state) in found { if !cities.iter().any(|(known, _)| *known == city) { cities.push((city, state)); } }
        cities
    }

    fn college(&self, text: &str) -> Option<String> {
        let found = self.college.find(text)?.as_str();
        let words: Vec<&str> = found.split_whitespace()
            .skip_while(|word| GENERIC_PREFIX.contains(&word.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase().as_str()))
            .collect();
        (words.len() >= 2).then(|| words.join(" ").trim_end_matches([',', '.', ';', ':']).to_string())
    }

    fn phone(&self, text: &str) -> Option<String> {
        self.phone.find_iter(text).find_map(|found| {
            let digits: String = found.as_str().chars().filter(char::is_ascii_digit).collect();
            let local = match digits.len() {
                10 => digits.as_str(),
                12 if digits.starts_with("91") => &digits[2..],
                _ => return None,
            };
            local.starts_with(['6', '7', '8', '9']).then(|| format!("91{local}"))
        })
    }

    fn vendor_name(&self, text: &str, sender: Option<&str>) -> Option<String> {
        const NOT_NAMES: &[&str] = &["Please", "Thanks", "Thank", "Regards", "Location", "Note", "Dm", "Interested", "Urgent"];
        let acceptable = |name: &str| !NOT_NAMES.iter().any(|word| name.starts_with(word)) && self.cities(name).is_empty();
        if let Some(captures) = self.contact.captures(text) && acceptable(&captures[1]) { return Some(captures[1].to_string()); }
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 && self.phone(line).is_some() && self.name_line.is_match(lines[index - 1]) && acceptable(lines[index - 1]) {
                return Some(lines[index - 1].to_string());
            }
        }
        sender.map(|sender| sender.chars().filter(|character| !character.is_ascii_digit() && *character != '+').collect::<String>())
            .map(|name| name.trim().trim_end_matches(':').trim().to_string())
            .filter(|name| name.chars().any(char::is_alphabetic))
    }

    fn rate(&self, text: &str) -> Option<String> {
        let plausible = |value: &str| rate_amount(value).is_some_and(|amount| (100.0..10_000_000.0).contains(&amount));
        self.rate.find_iter(text).map(|found| found.as_str().trim().to_string()).find(|value| plausible(value))
            .or_else(|| self.rate_label.captures(text).map(|captures| captures[1].trim().to_string())
                .filter(|value| value.chars().any(|character| character.is_ascii_digit())))
    }

    fn mode(&self, text: &str) -> Option<Mode> {
        let (online, offline) = (self.online.is_match(text), self.offline.is_match(text));
        if self.hybrid.is_match(text) || (online && offline) { Some(Mode::Hybrid) }
        else if online { Some(Mode::Online) }
        else if offline { Some(Mode::Offline) }
        else { None }
    }

    fn audience(&self, text: &str) -> Option<String> {
        let program = |value: Option<regex::Match>| value.map(|value| {
            let lower = value.as_str().to_lowercase();
            if lower.starts_with("eng") { " Engg".to_string() } else { format!(" {}", value.as_str().to_uppercase()) }
        }).unwrap_or_default();
        if let Some(captures) = self.semester.captures(text) {
            let number: u32 = captures[1].parse().unwrap_or(0);
            let suffix = match number { 1 => "st", 2 => "nd", 3 => "rd", _ => "th" };
            return Some(format!("{number}{suffix} sem{}", program(captures.get(2))));
        }
        if let Some(captures) = self.study_year.captures(text) {
            let year = captures[1].to_string();
            let mut year = year.chars();
            let year = year.next().map(|first| first.to_uppercase().chain(year).collect::<String>()).unwrap_or_default();
            return Some(format!("{year} year{}", program(captures.get(2))));
        }
        let lower = text.to_lowercase();
        if lower.contains("fdp") || lower.contains("faculty") { return Some("Faculty".into()); }
        if lower.contains("fresher") { return Some("Freshers".into()); }
        if lower.contains("corporate") || lower.contains("employees") || lower.contains("working professionals") {
            return Some("Corporate professionals".into());
        }
        self.degree.find(text).map(|degree| format!("{} students", degree.as_str().to_uppercase()))
    }

    fn logistics(&self, text: &str) -> (Provision, Provision, Provision, Option<String>) {
        use Provision::*;
        if self.no_tfa.is_match(text) { return (NotProvided, NotProvided, NotProvided, Some("No TFA".into())); }
        if self.tfa.is_match(text) { return (Provided, Provided, Provided, Some("TFA provided".into())); }
        let mut items = [Unknown; 3];
        let mut note = None;
        for line in text.lines() {
            let lower = line.to_lowercase();
            let present = [
                ["travel", "transport", "flight", "train ticket"].iter().any(|word| lower.contains(word)),
                ["food", "meal", "lunch"].iter().any(|word| lower.contains(word)),
                ["accommodation", "stay", "lodging", "hotel", "boarding"].iter().any(|word| lower.contains(word)),
            ];
            if !present.iter().any(|item| *item) { continue; }
            let words: Vec<&str> = lower.split(|character: char| !character.is_alphanumeric()).collect();
            let value = if words.iter().any(|word| ["no", "not", "own", "self", "excluded", "without"].contains(word)) {
                NotProvided
            } else if ["provided", "covered", "included", "arranged", "reimburs", "taken care", "free", "paid by"].iter().any(|word| lower.contains(word)) {
                Provided
            } else {
                continue;
            };
            for (item, mentioned) in items.iter_mut().zip(present) {
                if mentioned && *item == Unknown { *item = value; }
            }
            note.get_or_insert_with(|| line.trim().chars().take(100).collect::<String>());
        }
        (items[0], items[1], items[2], note)
    }
}

fn rate_amount(value: &str) -> Option<f64> {
    let lower = value.to_lowercase();
    let rest = &lower[lower.find(|character: char| character.is_ascii_digit())?..];
    let end = rest.find(|character: char| !(character.is_ascii_digit() || character == ',' || character == '.')).unwrap_or(rest.len());
    let number: f64 = rest[..end].replace(',', "").trim_end_matches('.').parse().ok()?;
    Some(if rest[end..].trim_start().starts_with('k') { number * 1000.0 } else { number })
}

fn summary(tech: &[String], city: Option<&str>, start: Option<NaiveDate>, end: Option<NaiveDate>) -> String {
    let subject = if tech.is_empty() { "Training requirement".to_string() } else { format!("{} training", tech.join(", ")) };
    let place = city.map(|city| format!(" in {city}")).unwrap_or_default();
    let when = match (start, end) {
        (Some(start), Some(end)) if start != end => format!(", {} to {}", start.format("%-d %b"), end.format("%-d %b %Y")),
        (Some(start), _) => format!(", from {}", start.format("%-d %b %Y")),
        _ => String::new(),
    };
    format!("{subject}{place}{when}")
}

#[async_trait]
impl LlmClient for RuleExtractor {
    async fn extract(&self, request: &ExtractionRequest) -> Result<String> {
        if request.screenshot.is_some() {
            return Err(Error::Configuration("screenshots need Groq; on-device rules read text only".into()));
        }
        let requirements = request.messages.iter().enumerate().flat_map(|(message_index, message)| {
            self.requirements(&message.raw_text, message.sender.as_deref(), request.today).into_iter()
                .map(move |requirement| ExtractionItem { message_index, requirement, raw_text: None })
        }).collect();
        Ok(serde_json::to_string(&ExtractionOutput { requirements })?)
    }
}

/// Uses the primary model when it works; otherwise falls back to on-device rules so nothing stalls.
pub struct FallbackExtractor<P> {
    primary: Option<P>,
    rules: Arc<RuleExtractor>,
    primary_failed: AtomicBool,
    local_batches: AtomicUsize,
    primary_error: Mutex<Option<String>>,
}

impl<P: LlmClient> FallbackExtractor<P> {
    pub fn new(primary: Option<P>, rules: Arc<RuleExtractor>) -> Self {
        Self { primary, rules, primary_failed: AtomicBool::new(false), local_batches: AtomicUsize::new(0), primary_error: Mutex::new(None) }
    }

    /// Human-readable name of the engine that produced the results of the last run.
    pub fn engine(&self) -> String {
        if self.primary.is_none() { return "on-device rules".into(); }
        if self.local_batches.load(Ordering::Relaxed) == 0 { return "Groq".into(); }
        let reason = self.primary_error.lock().ok().and_then(|error| error.clone()).unwrap_or_default();
        format!("on-device rules (Groq failed: {reason})")
    }

    fn fail(&self, reason: String) {
        self.primary_failed.store(true, Ordering::Relaxed);
        if let Ok(mut error) = self.primary_error.lock() { *error = Some(reason); }
    }
}

#[async_trait]
impl<P: LlmClient> LlmClient for FallbackExtractor<P> {
    async fn extract(&self, request: &ExtractionRequest) -> Result<String> {
        if let Some(primary) = &self.primary
            && (request.screenshot.is_some() || !self.primary_failed.load(Ordering::Relaxed))
        {
            match primary.extract(request).await {
                Ok(text) => match decode_output(&text, request) {
                    Ok(_) => return Ok(text),
                    Err(error) => self.fail(format!("output did not match the requirement format: {error}")),
                },
                Err(error) if request.screenshot.is_some() => return Err(error),
                Err(error) => self.fail(error.to_string()),
            }
        }
        self.local_batches.fetch_add(1, Ordering::Relaxed);
        self.rules.extract(request).await
    }
}
