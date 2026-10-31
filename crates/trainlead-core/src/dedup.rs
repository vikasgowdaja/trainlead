use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    models::{Message, Requirement},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DedupThresholds {
    pub location: f64,
    pub tech_jaccard: f64,
    pub raw_text: f64,
}

impl Default for DedupThresholds {
    fn default() -> Self {
        Self {
            location: 80.0,
            tech_jaccard: 0.5,
            raw_text: 70.0,
        }
    }
}

impl DedupThresholds {
    pub fn validate(&self) -> Result<()> {
        if !self.location.is_finite()
            || !(0.0..=100.0).contains(&self.location)
            || !self.raw_text.is_finite()
            || !(0.0..=100.0).contains(&self.raw_text)
            || !self.tech_jaccard.is_finite()
            || !(0.0..=1.0).contains(&self.tech_jaccard)
        {
            return Err(Error::Validation(
                "dedup scores must be 0-100; Jaccard must be 0-1".into(),
            ));
        }
        Ok(())
    }
}

pub fn normalized_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn exact_hash(message: &Message, requirement: &Requirement) -> Result<String> {
    let values = (
        normalized_text(&message.raw_text),
        requirement.college.as_deref().map(normalized_text),
        requirement.city.as_deref().map(normalized_text),
        requirement.start_date,
    );
    let bytes = serde_json::to_vec(&values)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn token_set_ratio(left: &str, right: &str) -> f64 {
    let left = normalized_text(left);
    let right = normalized_text(right);
    let left: BTreeSet<&str> = left.split_whitespace().collect();
    let right: BTreeSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let common: Vec<&str> = left.intersection(&right).copied().collect();
    let left_only: Vec<&str> = left.difference(&right).copied().collect();
    let right_only: Vec<&str> = right.difference(&left).copied().collect();
    let common_text = common.join(" ");
    let left_text = common
        .iter()
        .chain(&left_only)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let right_text = common
        .iter()
        .chain(&right_only)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let mut score = strsim::normalized_levenshtein(&left_text, &right_text);
    if !common.is_empty() {
        score = score
            .max(strsim::normalized_levenshtein(&common_text, &left_text))
            .max(strsim::normalized_levenshtein(&common_text, &right_text));
    }
    score * 100.0
}

pub fn is_similar(
    left: &Requirement,
    right: &Requirement,
    left_raw: &str,
    right_raw: &str,
    thresholds: &DedupThresholds,
) -> bool {
    let same_date = left.start_date.is_some() && left.start_date == right.start_date;
    let mut comparable = 0;
    let mut same_location = true;
    for (left_value, right_value) in [(&left.college, &right.college), (&left.city, &right.city)] {
        if let (Some(left_value), Some(right_value)) = (left_value, right_value) {
            comparable += 1;
            same_location &= token_set_ratio(left_value, right_value) >= thresholds.location;
        }
    }
    same_location &= comparable > 0;
    let left_tech: BTreeSet<String> = left
        .tech_stack
        .iter()
        .map(|tech| tech.to_lowercase())
        .collect();
    let right_tech: BTreeSet<String> = right
        .tech_stack
        .iter()
        .map(|tech| tech.to_lowercase())
        .collect();
    let union = left_tech.union(&right_tech).count();
    let same_tech = union > 0
        && left_tech.intersection(&right_tech).count() as f64 / union as f64
            >= thresholds.tech_jaccard;
    let signals = usize::from(same_date) + usize::from(same_location) + usize::from(same_tech);
    signals == 3 || (signals == 2 && token_set_ratio(left_raw, right_raw) >= thresholds.raw_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_sets_ignore_order_and_repetitions() {
        assert_eq!(
            token_set_ratio(
                "Mysuru engineering college",
                "college Mysuru engineering engineering"
            ),
            100.0
        );
        assert_eq!(token_set_ratio("", "Mysuru"), 0.0);
        assert_eq!(token_set_ratio("Mysuru", "Mysuru city"), 100.0);
    }

    #[test]
    fn thresholds_reject_nan_and_out_of_range() {
        assert!(
            DedupThresholds {
                location: f64::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            DedupThresholds {
                tech_jaccard: 1.1,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
