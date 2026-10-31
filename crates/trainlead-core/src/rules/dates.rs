use chrono::{Datelike, Duration, NaiveDate};
use regex::{Captures, Regex};

use super::compile;
use crate::Result;

const MONTH: &str = r"(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\b\.?";
const DAY: &str = r"(\d{1,2})(?:st|nd|rd|th)?";
const TO: &str = r"\s*(?:-|–|—|to|till|until|through|thru|&|and)\s*";
const YEAR: &str = r"(?:,?\s*(\d{4}))?";
/// Dates older than this without an explicit year are assumed to mean next year.
const PAST_GRACE_DAYS: i64 = 60;

#[derive(Debug, Clone, Copy)]
struct Parts { day: u32, month: u32, year: Option<i32> }

#[derive(Debug, Clone, Copy)]
enum Kind { Range(Parts, Parts), Single(Parts) }

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Schedule {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub payment: Option<NaiveDate>,
}

pub(super) struct DateParser {
    ranges: [Regex; 3],
    singles: [Regex; 3],
}

fn month(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter().position(|prefix| lower.starts_with(prefix)).map(|index| index as u32 + 1)
}

fn number<T: std::str::FromStr>(captures: &Captures, index: usize) -> Option<T> {
    captures.get(index).and_then(|value| value.as_str().parse().ok())
}

fn year(captures: &Captures, index: usize) -> Option<i32> {
    number::<i32>(captures, index).map(|year| if year < 100 { 2000 + year } else { year })
}

fn resolve(parts: Parts, today: NaiveDate) -> Option<NaiveDate> {
    if let Some(year) = parts.year {
        return (2000..2100).contains(&year).then(|| NaiveDate::from_ymd_opt(year, parts.month, parts.day)).flatten();
    }
    let date = NaiveDate::from_ymd_opt(today.year(), parts.month, parts.day)?;
    if date < today - Duration::days(PAST_GRACE_DAYS) {
        NaiveDate::from_ymd_opt(today.year() + 1, parts.month, parts.day)
    } else {
        Some(date)
    }
}

/// Resolves `parts` in the same year as `anchor`, moving to the next year if it would fall before it.
fn after(parts: Parts, anchor: NaiveDate) -> Option<NaiveDate> {
    if let Some(year) = parts.year { return NaiveDate::from_ymd_opt(year, parts.month, parts.day); }
    let date = NaiveDate::from_ymd_opt(anchor.year(), parts.month, parts.day)?;
    if date < anchor { NaiveDate::from_ymd_opt(anchor.year() + 1, parts.month, parts.day) } else { Some(date) }
}

impl DateParser {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            ranges: [
                compile(&format!(r"(?i)\b{DAY}{TO}{DAY}\s*(?:of\s+)?{MONTH}{YEAR}"))?,
                compile(&format!(r"(?i)\b{DAY}\s*(?:of\s+)?{MONTH}{YEAR}{TO}{DAY}\s*(?:of\s+)?{MONTH}{YEAR}"))?,
                compile(&format!(r"(?i)\b{MONTH}\s*{DAY}{TO}{DAY}\b{YEAR}"))?,
            ],
            singles: [
                compile(&format!(r"(?i)\b{DAY}\s*(?:of\s+)?{MONTH}{YEAR}"))?,
                compile(&format!(r"(?i)\b{MONTH}\s*{DAY}\b{YEAR}"))?,
                compile(r"\b(\d{1,2})[/.-](\d{1,2})[/.-](\d{4}|\d{2})\b")?,
            ],
        })
    }

    fn range(index: usize, captures: &Captures) -> Option<Kind> {
        let (first, second) = match index {
            0 => {
                let (month, year) = (month(&captures[3])?, year(captures, 4));
                (Parts { day: number(captures, 1)?, month, year }, Parts { day: number(captures, 2)?, month, year })
            }
            1 => {
                let second = Parts { day: number(captures, 4)?, month: month(&captures[5])?, year: year(captures, 6) };
                let mut first = Parts { day: number(captures, 1)?, month: month(&captures[2])?, year: year(captures, 3) };
                if first.year.is_none() {
                    first.year = second.year.map(|year| if first.month > second.month { year - 1 } else { year });
                }
                (first, second)
            }
            _ => {
                let (month, year) = (month(&captures[1])?, year(captures, 4));
                (Parts { day: number(captures, 2)?, month, year }, Parts { day: number(captures, 3)?, month, year })
            }
        };
        Some(Kind::Range(first, second))
    }

    fn single(index: usize, captures: &Captures) -> Option<Kind> {
        Some(Kind::Single(match index {
            0 => Parts { day: number(captures, 1)?, month: month(&captures[2])?, year: year(captures, 3) },
            1 => Parts { day: number(captures, 2)?, month: month(&captures[1])?, year: year(captures, 3) },
            _ => Parts { day: number(captures, 1)?, month: number(captures, 2)?, year: year(captures, 3) },
        }))
    }

    /// Every date mention with the lowercase text preceding it on the same line.
    fn mentions(&self, text: &str) -> Vec<(usize, Kind, String)> {
        let mut taken: Vec<(usize, usize)> = Vec::new();
        let mut found = Vec::new();
        let patterns = self.ranges.iter().enumerate().map(|(index, regex)| (regex, index, true))
            .chain(self.singles.iter().enumerate().map(|(index, regex)| (regex, index, false)));
        for (regex, index, is_range) in patterns {
            for captures in regex.captures_iter(text) {
                let Some(whole) = captures.get(0) else { continue };
                if taken.iter().any(|(start, end)| whole.start() < *end && *start < whole.end()) { continue; }
                let kind = if is_range { Self::range(index, &captures) } else { Self::single(index, &captures) };
                let Some(kind) = kind else { continue };
                taken.push((whole.start(), whole.end()));
                let line_start = text[..whole.start()].rfind('\n').map_or(0, |position| position + 1);
                found.push((whole.start(), kind, text[line_start..whole.start()].to_lowercase()));
            }
        }
        found.sort_by_key(|(position, _, _)| *position);
        found
    }

    pub(super) fn schedule(&self, text: &str, today: NaiveDate) -> Schedule {
        let has = |context: &str, words: &[&str]| words.iter().any(|word| context.contains(word));
        let mut schedule = Schedule::default();
        let mut payment = None;
        let mut singles: Vec<(Parts, &str)> = Vec::new();
        let mentions = self.mentions(text);
        for (_, kind, context) in &mentions {
            let is_payment = has(context, &["pay", "paid", "invoice", "settle", "credit", "release"]);
            match *kind {
                Kind::Single(parts) if is_payment => { payment.get_or_insert(parts); }
                Kind::Range(first, second) if !is_payment && schedule.start.is_none() => {
                    if let Some(start) = resolve(first, today) {
                        schedule.start = Some(start);
                        schedule.end = after(second, start);
                    }
                }
                Kind::Single(parts) => singles.push((parts, context)),
                Kind::Range(..) => {}
            }
        }
        if schedule.start.is_none() {
            let labelled = |words: &[&str]| singles.iter().find(|(_, context)| has(context, words)).map(|(parts, _)| *parts);
            let start = labelled(&["start", "from", "commenc", "begin", "date"]).or(singles.first().map(|(parts, _)| *parts));
            schedule.start = start.and_then(|parts| resolve(parts, today));
            if let Some(start) = schedule.start {
                let end = labelled(&["end", "till", "until", "complet"]).or(singles.get(1).map(|(parts, _)| *parts));
                schedule.end = end.and_then(|parts| after(parts, start))
                    .filter(|end| *end > start && *end - start <= Duration::days(60));
            }
        }
        schedule.payment = payment.and_then(|parts| match schedule.start {
            Some(start) => after(parts, start),
            None => resolve(parts, today),
        });
        schedule
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> Option<NaiveDate> { NaiveDate::from_ymd_opt(year, month, day) }

    fn parse(text: &str, today: Option<NaiveDate>) -> Schedule {
        DateParser::new().map(|parser| parser.schedule(text, today.unwrap_or_default())).unwrap_or_default()
    }

    #[test]
    fn reads_common_whatsapp_date_formats() {
        let today = date(2026, 10, 31);
        let expect = |text: &str, start, end| {
            let schedule = parse(text, today);
            assert_eq!((schedule.start, schedule.end), (start, end), "{text}");
        };
        expect("AngularJS, Mysuru, 2-7 Nov, 6th sem", date(2026, 11, 2), date(2026, 11, 7));
        expect("2nd to 7th November 2026", date(2026, 11, 2), date(2026, 11, 7));
        expect("Angular instructor, November 2 through 7.", date(2026, 11, 2), date(2026, 11, 7));
        expect("Dates: 28 Dec - 3 Jan", date(2026, 12, 28), date(2027, 1, 3));
        expect("Start Date: 1 November 2026\nEnd Date: 5 November 2026", date(2026, 11, 1), date(2026, 11, 5));
        expect("Training on 12/11/2026", date(2026, 11, 12), None);
        expect("Batch starts 5 Jan", date(2027, 1, 5), None);
    }

    #[test]
    fn separates_payment_dates_from_the_schedule() {
        let schedule = parse("Angular, 2-7 Nov, TFA provided, payment 25 Nov", date(2026, 10, 31));
        assert_eq!(schedule, Schedule { start: date(2026, 11, 2), end: date(2026, 11, 7), payment: date(2026, 11, 25) });
        let crossing = parse("Training 28-30 Dec\nPayment on 10 Jan", date(2026, 12, 1));
        assert_eq!(crossing.payment, date(2027, 1, 10));
    }
}
