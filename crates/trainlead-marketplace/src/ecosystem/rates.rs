use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RateUnit { Day, Hour, Session, Total }

impl RateUnit {
    pub fn as_str(self) -> &'static str {
        match self { Self::Day => "day", Self::Hour => "hour", Self::Session => "session", Self::Total => "total" }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value { "day" => Some(Self::Day), "hour" => Some(Self::Hour), "session" => Some(Self::Session), "total" => Some(Self::Total), _ => None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Rate {
    pub amount_inr: i64,
    pub unit: Option<RateUnit>,
}

const MAX_RATE_INR: i64 = 100_000_000;

/// Best-effort parse of free-text rates such as "₹8,000/day", "8k per day" or "1.5L total".
pub fn parse_rate(text: &str) -> Option<Rate> {
    let lower = text.to_lowercase().replace(',', "");
    let start = lower.find(|character: char| character.is_ascii_digit())?;
    let digits_end = lower[start..].find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .map_or(lower.len(), |offset| start + offset);
    let number: f64 = lower[start..digits_end].trim_end_matches('.').parse().ok()?;
    let rest = lower[digits_end..].trim_start();
    let suffix = |prefix: &str| rest.strip_prefix(prefix)
        .is_some_and(|after| !after.starts_with(|character: char| character.is_alphabetic()));
    let multiplier = if rest.starts_with("lakh") || rest.starts_with("lac") || suffix("l") {
        100_000.0
    } else if suffix("k") {
        1_000.0
    } else {
        1.0
    };
    let amount = (number * multiplier).round();
    if !(1.0..=MAX_RATE_INR as f64).contains(&amount) { return None; }
    let units = [
        (RateUnit::Hour, &["hour", "hr"][..]),
        (RateUnit::Day, &["day", "pd"][..]),
        (RateUnit::Session, &["session"][..]),
        (RateUnit::Total, &["total", "lump", "package", "overall"][..]),
    ];
    let unit = units.iter()
        .filter_map(|(unit, patterns)| patterns.iter().filter_map(|pattern| rest.find(pattern)).min().map(|position| (position, *unit)))
        .min_by_key(|(position, _)| *position)
        .map(|(_, unit)| unit);
    Some(Rate { amount_inr: amount as i64, unit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_indian_rate_formats() {
        assert_eq!(parse_rate("₹8,000/day"), Some(Rate { amount_inr: 8000, unit: Some(RateUnit::Day) }));
        assert_eq!(parse_rate("8k per day"), Some(Rate { amount_inr: 8000, unit: Some(RateUnit::Day) }));
        assert_eq!(parse_rate("Rs 1200 per hour, 6 hrs a day"), Some(Rate { amount_inr: 1200, unit: Some(RateUnit::Hour) }));
        assert_eq!(parse_rate("1.5L total"), Some(Rate { amount_inr: 150_000, unit: Some(RateUnit::Total) }));
        assert_eq!(parse_rate("9000"), Some(Rate { amount_inr: 9000, unit: None }));
        assert_eq!(parse_rate("As per industry standards"), None);
        assert_eq!(parse_rate("0 per day"), None);
    }
}
