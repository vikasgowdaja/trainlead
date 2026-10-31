use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use trainlead_core::{Error, Result, models::{Mode, normalize_tech}};

use super::Status;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VendorProfile {
    pub legal_name: String,
    pub display_name: Option<String>,
    pub gstin: Option<String>,
    pub pan: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub pincode: Option<String>,
    pub website: Option<String>,
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
    /// Numbers this vendor posts from in WhatsApp groups; used to link their postings.
    pub whatsapp_numbers: Vec<String>,
    pub domains: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Travel { #[default] LocalOnly, WithinState, Anywhere }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillEntry {
    pub name: String,
    pub proficiency: u8,
    #[serde(default)]
    pub years: Option<u32>,
    #[serde(default)]
    pub last_used: Option<i32>,
    #[serde(default)]
    pub trainings_delivered: Option<u32>,
    #[serde(default)]
    pub evidence_url: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unavailable {
    pub from: NaiveDate,
    pub to: NaiveDate,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrainerProfile {
    pub name: String,
    pub city: Option<String>,
    pub state: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub languages: Vec<String>,
    pub experience_years: Option<u32>,
    /// Per-day rate range in INR.
    pub rate_min: Option<u32>,
    pub rate_max: Option<u32>,
    pub travel: Travel,
    pub modes: Vec<Mode>,
    pub bio: Option<String>,
    pub skills: Vec<SkillEntry>,
    pub unavailable: Vec<Unavailable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub label: &'static str,
    pub done: bool,
    pub required: bool,
}

fn invalid(message: impl Into<String>) -> Error { Error::Validation(message.into()) }

fn text(value: &mut Option<String>, field: &str, max: usize) -> Result<()> {
    *value = value.take().map(|text| text.trim().to_string()).filter(|text| !text.is_empty());
    if value.as_ref().is_some_and(|text| text.chars().count() > max) {
        return Err(invalid(format!("{field} must be at most {max} characters")));
    }
    Ok(())
}

fn required(value: &mut String, field: &str, max: usize) -> Result<()> {
    *value = value.trim().to_string();
    if value.is_empty() { return Err(invalid(format!("{field} is required"))); }
    if value.chars().count() > max { return Err(invalid(format!("{field} must be at most {max} characters"))); }
    Ok(())
}

fn list(values: &mut Vec<String>, field: &str, max_items: usize) -> Result<()> {
    let mut cleaned: Vec<String> = Vec::new();
    for value in values.drain(..) {
        let value = value.trim().to_string();
        if value.is_empty() || cleaned.iter().any(|known| known.eq_ignore_ascii_case(&value)) { continue; }
        if value.chars().count() > 80 { return Err(invalid(format!("each {field} entry must be at most 80 characters"))); }
        cleaned.push(value);
    }
    if cleaned.len() > max_items { return Err(invalid(format!("at most {max_items} {field} entries"))); }
    *values = cleaned;
    Ok(())
}

/// Digits with country code; 10-digit Indian mobile numbers get 91 prepended.
pub fn normalize_phone(value: &str) -> Result<String> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    let digits = if digits.len() == 10 && digits.starts_with(['6', '7', '8', '9']) { format!("91{digits}") } else { digits };
    if !(7..=15).contains(&digits.len()) { return Err(invalid(format!("{value} is not a valid phone number"))); }
    Ok(digits)
}

fn phone(value: &mut Option<String>) -> Result<()> {
    text(value, "Phone", 30)?;
    if let Some(number) = value.as_deref() { *value = Some(normalize_phone(number)?); }
    Ok(())
}

fn email(value: &mut Option<String>) -> Result<()> {
    text(value, "Email", 120)?;
    if let Some(address) = value.as_deref() {
        let valid = address.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        }) && !address.contains(char::is_whitespace) && address.matches('@').count() == 1;
        if !valid { return Err(invalid(format!("{address} is not a valid email address"))); }
        *value = Some(address.to_lowercase());
    }
    Ok(())
}

pub fn validate_url(value: &mut Option<String>, field: &str) -> Result<()> {
    text(value, field, 300)?;
    if let Some(url) = value.as_deref()
        && (!(url.starts_with("https://") || url.starts_with("http://")) || url.contains(char::is_whitespace))
    {
        return Err(invalid(format!("{field} must be a web address starting with https://")));
    }
    Ok(())
}

fn code(value: &mut Option<String>, field: &str, valid: fn(&[u8]) -> bool) -> Result<()> {
    text(value, field, 20)?;
    if let Some(raw) = value.as_deref() {
        let upper = raw.to_uppercase().replace(' ', "");
        if !valid(upper.as_bytes()) { return Err(invalid(format!("{raw} is not a valid {field}"))); }
        *value = Some(upper);
    }
    Ok(())
}

fn pan_shape(bytes: &[u8]) -> bool {
    bytes.len() == 10 && bytes[..5].iter().all(u8::is_ascii_uppercase)
        && bytes[5..9].iter().all(u8::is_ascii_digit) && bytes[9].is_ascii_uppercase()
}

/// 2-digit state code, PAN, entity number, 'Z', checksum character.
fn gstin_shape(bytes: &[u8]) -> bool {
    bytes.len() == 15 && bytes[..2].iter().all(u8::is_ascii_digit) && pan_shape(&bytes[2..12])
        && ((bytes[12].is_ascii_digit() && bytes[12] != b'0') || bytes[12].is_ascii_uppercase())
        && bytes[13] == b'Z' && bytes[14].is_ascii_alphanumeric()
}

fn pincode_shape(bytes: &[u8]) -> bool { bytes.len() == 6 && bytes[0] != b'0' && bytes.iter().all(u8::is_ascii_digit) }

impl VendorProfile {
    pub fn normalize(&mut self) -> Result<()> {
        required(&mut self.legal_name, "Legal name", 160)?;
        text(&mut self.display_name, "Display name", 160)?;
        code(&mut self.gstin, "GSTIN", gstin_shape)?;
        code(&mut self.pan, "PAN", pan_shape)?;
        if let (Some(gstin), Some(pan)) = (&self.gstin, &self.pan) && &gstin[2..12] != pan.as_str() {
            return Err(invalid("PAN does not match the PAN inside the GSTIN"));
        }
        text(&mut self.address, "Address", 300)?;
        text(&mut self.city, "City", 80)?;
        text(&mut self.state, "State", 80)?;
        code(&mut self.pincode, "PIN code", pincode_shape)?;
        validate_url(&mut self.website, "Website")?;
        text(&mut self.contact_name, "Contact person", 120)?;
        phone(&mut self.contact_phone)?;
        email(&mut self.contact_email)?;
        self.whatsapp_numbers = self.whatsapp_numbers.iter().filter(|number| !number.trim().is_empty())
            .map(|number| normalize_phone(number)).collect::<Result<_>>()?;
        self.whatsapp_numbers.dedup();
        list(&mut self.whatsapp_numbers, "WhatsApp number", 10)?;
        list(&mut self.domains, "domain", 30)?;
        text(&mut self.notes, "Notes", 2000)
    }

    pub fn phones(&self) -> impl Iterator<Item = &str> {
        self.contact_phone.iter().map(String::as_str).chain(self.whatsapp_numbers.iter().map(String::as_str))
    }

    pub fn checklist(&self, documents: usize, status: Status) -> Vec<Step> {
        vec![
            Step { label: "Company profile", done: !self.legal_name.is_empty() && self.city.is_some() && self.state.is_some(), required: true },
            Step { label: "Contact person", done: self.contact_name.is_some() && (self.contact_phone.is_some() || self.contact_email.is_some()), required: true },
            Step { label: "Domains served", done: !self.domains.is_empty(), required: true },
            Step { label: "Documents", done: documents > 0, required: false },
            Step { label: "Submitted for review", done: status != Status::Draft, required: false },
            Step { label: "Approved", done: status == Status::Approved, required: false },
        ]
    }
}

impl TrainerProfile {
    pub fn normalize(&mut self) -> Result<()> {
        required(&mut self.name, "Name", 120)?;
        text(&mut self.city, "City", 80)?;
        text(&mut self.state, "State", 80)?;
        phone(&mut self.phone)?;
        email(&mut self.email)?;
        list(&mut self.languages, "language", 12)?;
        if self.experience_years.is_some_and(|years| years > 60) { return Err(invalid("Experience must be at most 60 years")); }
        if let (Some(min), Some(max)) = (self.rate_min, self.rate_max) && min > max {
            return Err(invalid("Minimum rate cannot be higher than maximum rate"));
        }
        if [self.rate_min, self.rate_max].into_iter().flatten().any(|rate| rate == 0 || rate > 1_000_000) {
            return Err(invalid("Rates must be between ₹1 and ₹10,00,000 per day"));
        }
        self.modes.sort_by_key(|mode| *mode as u8);
        self.modes.dedup();
        text(&mut self.bio, "Bio", 2000)?;
        if self.skills.len() > 40 { return Err(invalid("At most 40 skills")); }
        let mut seen: Vec<String> = Vec::new();
        for skill in &mut self.skills {
            skill.name = normalize_tech(skill.name.trim());
            if skill.name.is_empty() { return Err(invalid("Every skill needs a name")); }
            if seen.iter().any(|known| known.eq_ignore_ascii_case(&skill.name)) {
                return Err(invalid(format!("{} is listed twice", skill.name)));
            }
            seen.push(skill.name.clone());
            if !(1..=5).contains(&skill.proficiency) { return Err(invalid(format!("{} proficiency must be 1-5", skill.name))); }
            if skill.years.is_some_and(|years| years > 60) { return Err(invalid(format!("{} years must be at most 60", skill.name))); }
            if skill.last_used.is_some_and(|year| !(1970..=2100).contains(&year)) {
                return Err(invalid(format!("{} last-used year looks wrong", skill.name)));
            }
            validate_url(&mut skill.evidence_url, "Evidence link")?;
        }
        if self.unavailable.len() > 100 { return Err(invalid("At most 100 unavailable periods")); }
        for block in &mut self.unavailable {
            if block.to < block.from { return Err(invalid("An unavailable period ends before it starts")); }
            text(&mut block.note, "Note", 120)?;
        }
        self.unavailable.sort_by_key(|block| block.from);
        Ok(())
    }

    pub fn checklist(&self, document_kinds: &[String], status: Status) -> Vec<Step> {
        vec![
            Step { label: "Profile", done: !self.name.is_empty() && self.city.is_some() && (self.phone.is_some() || self.email.is_some())
                && self.experience_years.is_some() && !self.modes.is_empty(), required: true },
            Step { label: "Day rate", done: self.rate_min.is_some(), required: true },
            Step { label: "Skills", done: !self.skills.is_empty(), required: true },
            Step { label: "Resume uploaded", done: document_kinds.iter().any(|kind| kind == "Resume"), required: true },
            Step { label: "Availability", done: !self.unavailable.is_empty(), required: false },
            Step { label: "Submitted for review", done: status != Status::Draft, required: false },
            Step { label: "Verified", done: status == Status::Approved, required: false },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_normalizes_indian_business_identifiers() -> Result<()> {
        let mut vendor = VendorProfile {
            legal_name: " Asha Trainings Pvt Ltd ".into(), gstin: Some("29abcde1234f1z5".into()), pan: Some("ABCDE1234F".into()),
            pincode: Some("570001".into()), contact_phone: Some("+91 98450 12345".into()), contact_email: Some("Ops@Asha.in".into()),
            whatsapp_numbers: vec!["9845012345".into(), "".into()], domains: vec!["Java".into(), "java".into(), " ".into()],
            ..Default::default()
        };
        vendor.normalize()?;
        assert_eq!(vendor.legal_name, "Asha Trainings Pvt Ltd");
        assert_eq!(vendor.gstin.as_deref(), Some("29ABCDE1234F1Z5"));
        assert_eq!(vendor.contact_phone.as_deref(), Some("919845012345"));
        assert_eq!(vendor.contact_email.as_deref(), Some("ops@asha.in"));
        assert_eq!((vendor.whatsapp_numbers.clone(), vendor.domains.clone()), (vec!["919845012345".to_string()], vec!["Java".to_string()]));
        for (field, value) in [("gstin", "29ABCDE1234F1Y5"), ("pan", "ABCD1234F"), ("pincode", "057000"), ("email", "a@b")] {
            let mut bad = VendorProfile { legal_name: "X".into(), ..Default::default() };
            match field {
                "gstin" => bad.gstin = Some(value.into()),
                "pan" => bad.pan = Some(value.into()),
                "pincode" => bad.pincode = Some(value.into()),
                _ => bad.contact_email = Some(value.into()),
            }
            assert!(bad.normalize().is_err(), "accepted {field}={value}");
        }
        let mut mismatch = VendorProfile { legal_name: "X".into(), gstin: Some("29ABCDE1234F1Z5".into()), pan: Some("ZZZZZ9999Z".into()), ..Default::default() };
        assert!(mismatch.normalize().is_err());
        Ok(())
    }

    #[test]
    fn trainer_skills_are_normalized_and_bounded() -> Result<()> {
        let skill = |name: &str, proficiency| SkillEntry {
            name: name.into(), proficiency, years: Some(3), last_used: Some(2026), trainings_delivered: Some(12),
            evidence_url: None, verified: false,
        };
        let mut trainer = TrainerProfile { name: "Ravi".into(), skills: vec![skill("angular js", 4)], rate_min: Some(6000), rate_max: Some(9000), ..Default::default() };
        trainer.normalize()?;
        assert_eq!(trainer.skills[0].name, "AngularJS");
        let mut duplicate = TrainerProfile { name: "Ravi".into(), skills: vec![skill("React", 3), skill("reactjs", 2)], ..Default::default() };
        assert!(duplicate.normalize().is_err());
        let mut proficiency = TrainerProfile { name: "Ravi".into(), skills: vec![skill("Java", 6)], ..Default::default() };
        assert!(proficiency.normalize().is_err());
        let mut rates = TrainerProfile { name: "Ravi".into(), rate_min: Some(9000), rate_max: Some(6000), ..Default::default() };
        assert!(rates.normalize().is_err());
        Ok(())
    }
}
