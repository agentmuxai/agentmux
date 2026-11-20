// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which keys of `settings.json` AgentMux does not recognise, and what each was probably meant to be
//! (docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md §5.6, case 4).
//!
//! A key the schema does not list is kept in the file and has no effect. It is listed in the
//! Settings view with a "did you mean" when another key is a few characters away (a typo), so the
//! person can fix or remove it. The app never deletes one on its own.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// One key of `settings.json` that is not a setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnrecognizedSetting {
    pub key: String,
    /// The recognised key it is closest to, when it is close enough to be a typo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Where the key is in the file (1-based), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Top-level members of `settings.json` that are not in the schema and are not mistakes: the per-connection
/// settings AgentMux writes (`connections.<name>`, typed in `SettingsType`), and the editor hint `$schema`.
const NOT_SETTINGS: [&str; 2] = ["connections", "$schema"];

/// How many single-character edits away a key may be from a recognised one to be offered as it:
/// one for a short key, up to three for a long one.
fn allowed_distance(len: usize) -> usize {
    (len / 8).clamp(1, 3)
}

/// The edit distance between `a` and `b` (insert, delete, replace, one character each), or `None`
/// when it is more than `max`. Works on characters, so a non-ASCII key is not cut in half.
fn distance(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let replace = previous[j] + usize::from(ca != cb);
            row.push(replace.min(previous[j + 1] + 1).min(row[j] + 1));
        }
        previous = row;
    }
    let d = previous[b.len()];
    (d <= max).then_some(d)
}

/// The recognised key `key` is closest to, if it is close enough: a key that differs only in case
/// is always offered. Ties go to the key that sorts first, so the answer does not depend on map order.
fn closest<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    let lower: Vec<char> = key.to_lowercase().chars().collect();
    let max = allowed_distance(lower.len());
    known
        .iter()
        .filter(|k| !k.ends_with('*'))
        .filter_map(|k| {
            let other: Vec<char> = k.to_lowercase().chars().collect();
            distance(&lower, &other, max).map(|d| (d, *k))
        })
        .min()
        .map(|(_, k)| k)
}

/// The keys of `keys` that `known` (the schema's keys) does not list, in the order the file has them
/// when `lines` says where each is, otherwise by name. `lines` maps a key to its 1-based line.
pub fn find<'a>(keys: impl IntoIterator<Item = &'a String>, known: &[&str], lines: &HashMap<String, u32>) -> Vec<UnrecognizedSetting> {
    let mut found: Vec<UnrecognizedSetting> = keys
        .into_iter()
        .filter(|key| !known.contains(&key.as_str()) && !NOT_SETTINGS.contains(&key.as_str()))
        .map(|key| UnrecognizedSetting { key: key.clone(), suggestion: closest(key, known).map(str::to_string), line: lines.get(key).copied() })
        .collect();
    found.sort_by(|a, b| (a.line.unwrap_or(u32::MAX), &a.key).cmp(&(b.line.unwrap_or(u32::MAX), &b.key)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: [&str; 6] = ["term:fontsize", "term:theme", "term:scrollback", "app:*", "window:zoom", "agent:livefeedturns"];

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn found(list: &[&str]) -> Vec<UnrecognizedSetting> {
        find(&keys(list), &KNOWN, &HashMap::new())
    }

    #[test]
    fn a_recognised_key_is_not_listed_and_a_family_key_is_recognised_as_written() {
        assert!(found(&["term:fontsize", "app:*", "window:zoom"]).is_empty());
    }

    #[test]
    fn what_agentmux_writes_into_the_file_beside_the_settings_is_not_listed() {
        assert!(found(&["connections", "$schema"]).is_empty());
    }

    #[test]
    fn a_typo_is_listed_with_the_key_it_was_probably_meant_to_be() {
        let list = found(&["term:fontsise"]);
        assert_eq!(list, vec![UnrecognizedSetting { key: "term:fontsise".into(), suggestion: Some("term:fontsize".into()), line: None }]);
    }

    #[test]
    fn a_key_that_differs_only_in_case_is_offered_as_the_right_one() {
        assert_eq!(found(&["Term:FontSize"])[0].suggestion.as_deref(), Some("term:fontsize"));
    }

    #[test]
    fn a_key_far_from_everything_is_listed_with_no_suggestion() {
        assert_eq!(found(&["my:own:note"])[0].suggestion, None);
        assert_eq!(found(&["zzzzzz"])[0].suggestion, None);
    }

    #[test]
    fn a_short_key_is_offered_only_one_edit_away_and_a_long_one_up_to_three() {
        // "app:*" is a family and never offered; a two-edit typo of a short key is not close enough.
        assert_eq!(found(&["term:thme"])[0].suggestion.as_deref(), Some("term:theme"));
        assert_eq!(found(&["term:tm"])[0].suggestion, None);
        assert_eq!(found(&["agent:livefeedturnz"])[0].suggestion.as_deref(), Some("agent:livefeedturns"));
        assert_eq!(found(&["agent:livefedtrns"])[0].suggestion.as_deref(), Some("agent:livefeedturns"));
    }

    #[test]
    fn a_family_key_is_never_the_suggestion() {
        assert_eq!(found(&["app:x"])[0].suggestion, None);
    }

    #[test]
    fn the_answer_follows_the_file_order_then_the_name_and_does_not_depend_on_map_order() {
        let lines: HashMap<String, u32> = [("b:key".to_string(), 3), ("a:key".to_string(), 9)].into_iter().collect();
        let list = find(&keys(&["a:key", "b:key", "c:key"]), &KNOWN, &lines);
        assert_eq!(list.iter().map(|u| u.key.as_str()).collect::<Vec<_>>(), ["b:key", "a:key", "c:key"]);
        assert_eq!(list[0].line, Some(3));
        assert_eq!(list[2].line, None);
    }

    #[test]
    fn a_non_ascii_key_is_compared_by_characters() {
        let list = found(&["term:théme"]);
        assert_eq!(list[0].suggestion.as_deref(), Some("term:theme"));
    }

    #[test]
    fn distance_stops_early_when_the_lengths_alone_are_too_far_apart() {
        let a: Vec<char> = "abc".chars().collect();
        let b: Vec<char> = "abcdefgh".chars().collect();
        assert_eq!(distance(&a, &b, 3), None);
        assert_eq!(distance(&a, &a, 3), Some(0));
    }
}
