use chrono::{NaiveDate, NaiveTime};
use regex::Regex;

use crate::{Error, Result, models::Message};

pub fn parse_export(text: &str, source_group: &str) -> Result<Vec<Message>> {
    let header = Regex::new(
        r"^\[?(\d{1,2}[/.]\d{1,2}[/.]\d{2,4}),?\s+(\d{1,2}:\d{2}(?::\d{2})?(?:\s*[aApP]\.?[mM]\.?)?)(?:\]\s*|\s+-\s+)(.*)$",
    ).map_err(|error| Error::Validation(error.to_string()))?;
    let mut messages: Vec<Message> = Vec::new();
    let mut current: Option<Message> = None;
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.replace(['\u{200e}', '\u{200f}', '\u{feff}'], "")
            .replace(['\u{202f}', '\u{a0}'], " ");
        if let Some(captures) = header.captures(&line) {
            if let Some(message) = current.take() { messages.push(message); }
            let timestamp = parse_timestamp(&captures[1], &captures[2])
                .map_err(|detail| Error::Export { line: index + 1, detail })?;
            if let Some((sender, body)) = captures[3].split_once(": ")
                && !sender.trim().is_empty()
            {
                current = Some(Message {
                    source_group: source_group.to_string(),
                    sender: Some(sender.trim().to_string()),
                    timestamp,
                    raw_text: body.to_string(),
                });
            }
        } else if let Some(message) = current.as_mut() {
            message.raw_text.push('\n');
            message.raw_text.push_str(&line);
        }
    }
    if let Some(message) = current { messages.push(message); }
    Ok(messages)
}

fn parse_timestamp(date: &str, time: &str) -> std::result::Result<String, String> {
    let date = date.replace('.', "/");
    let parts: Vec<&str> = date.split('/').collect();
    let [day, month, year] = parts.as_slice() else { return Err("invalid date".into()); };
    let day: u32 = day.parse().map_err(|_| "invalid day")?;
    let month: u32 = month.parse().map_err(|_| "invalid month")?;
    let mut year_number: i32 = year.parse().map_err(|_| "invalid year")?;
    if year.len() == 2 { year_number += 2000; }
    let date = NaiveDate::from_ymd_opt(year_number, month, day).ok_or("invalid calendar date")?;
    let time = time.replace('.', "").to_uppercase();
    let time = if time.contains("AM") || time.contains("PM") {
        let spaced = time.replace("AM", " AM").replace("PM", " PM");
        let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
        NaiveTime::parse_from_str(&spaced, "%I:%M:%S %p")
            .or_else(|_| NaiveTime::parse_from_str(&spaced, "%I:%M %p"))
    } else {
        NaiveTime::parse_from_str(&time, "%H:%M:%S")
            .or_else(|_| NaiveTime::parse_from_str(&time, "%H:%M"))
    }.map_err(|error| error.to_string())?;
    Ok(date.and_time(time).format("%Y-%m-%dT%H:%M:%S").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_formats_multiline_and_system_boundaries() -> Result<()> {
        let text = "12/10/26, 4:55 pm - Alice: Training requirement\nAngular JS\n12/10/26, 4:56 pm - Bob joined using an invite link\nnot a message\n[12/10/26, 4:55:10 pm] Bob: Workshop\nMysuru";
        let messages = parse_export(text, "Vendors")?;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, "2026-10-12T16:55:00");
        assert_eq!(messages[0].raw_text, "Training requirement\nAngular JS");
        assert_eq!(messages[1].timestamp, "2026-10-12T16:55:10");
        assert_eq!(messages[1].sender.as_deref(), Some("Bob"));
        assert_eq!(messages[1].raw_text, "Workshop\nMysuru");
        Ok(())
    }

    #[test]
    fn handles_unicode_spacing_24h_and_colons() -> Result<()> {
        let messages = parse_export("\u{feff}[2.11.2026, 09:05:01] Alice: Dates: 2-7 Nov\n[2.11.2026, 4:55\u{202f}p.m.] Bob: FDP", "FDP")?;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].raw_text, "Dates: 2-7 Nov");
        assert_eq!(messages[1].timestamp, "2026-11-02T16:55:00");
        Ok(())
    }

    #[test]
    fn rejects_bad_calendar_dates() {
        assert!(parse_export("31/02/26, 4:55 pm - Alice: Workshop", "Test").is_err());
    }
}