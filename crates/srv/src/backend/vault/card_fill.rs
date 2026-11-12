// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a whole-card fill puts in each field of a form
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! The host says what a field is (its slot) and how it takes text (a choice, a
//! length, a placeholder, a pattern); this turns the card's parts into the text that
//! field takes: an expiry as `MM/YY`, `MM/YYYY` or `MMYY`, a month and a year to
//! choose or type, a number grouped or plain, a code that is left alone when the
//! field is shorter than it. Pure functions: the values come in and go out as
//! `Zeroizing` text, and no reason a part is left empty ever carries one.

use zeroize::Zeroizing;

use super::card;
use super::seal::Parts;

/// What the host reports about a field it matched to a card slot.
#[derive(Debug, Clone, Default)]
pub struct FormField {
    pub slot: String,
    /// A `<select>`: the host picks the option the text names.
    pub select: bool,
    pub max_length: Option<usize>,
    pub placeholder: String,
    pub pattern: String,
}

/// What goes in a field.
pub enum Shaped {
    /// Typed (set) as it is.
    Text(Zeroizing<String>),
    /// A month (`03`) or a year (`2027`) for the host to find among a `<select>`'s options.
    Choose(Zeroizing<String>),
    /// Left empty, and why (never a value).
    Left(String),
}

/// The card part a slot takes, in the card's own names.
pub fn part_of_slot(slot: &str) -> Option<&'static str> {
    Some(match slot {
        "card-number" => "number",
        "card-expiry" => "expiry",
        "card-exp-month" => "exp_month",
        "card-exp-year" => "exp_year",
        "card-cvc" => "cvc",
        "card-name" => "name",
        "card-zip" => "zip",
        _ => return None,
    })
}

/// What the person reads for a slot's part in the answer and the approval: names only.
pub fn part_name(slot: &str) -> &'static str {
    match slot {
        "card-number" => "number",
        "card-expiry" | "card-exp-month" | "card-exp-year" => "expiry",
        "card-cvc" => "security code",
        "card-name" => "name",
        "card-zip" => "ZIP",
        _ => "card detail",
    }
}

fn shaped(s: String) -> Shaped {
    Shaped::Text(Zeroizing::new(s))
}

fn left(part: &str) -> Shaped {
    Shaped::Left(format!("the card has no {part} saved"))
}

/// The text for one field, from the card's parts.
pub fn shape(field: &FormField, parts: &Parts) -> Shaped {
    match field.slot.as_str() {
        "card-number" => number(field, parts),
        "card-expiry" => expiry(field, parts),
        "card-exp-month" => month(field, parts),
        "card-exp-year" => year(field, parts),
        "card-cvc" => code(field, parts),
        "card-name" => plain(parts, "name", "holder name"),
        "card-zip" => plain(parts, "zip", "billing ZIP"),
        other => Shaped::Left(format!("{other} isn't a card field")),
    }
}

fn plain(parts: &Parts, part: &str, what: &str) -> Shaped {
    match parts.get(part).filter(|v| !v.trim().is_empty()) {
        Some(v) => shaped(v.to_string()),
        None => left(what),
    }
}

fn number(field: &FormField, parts: &Parts) -> Shaped {
    let Some(digits) = parts.get("number").and_then(card::normalize_number) else {
        return left("number");
    };
    let grouped = card::group(&digits);
    // A field that has room for the spaces and shows them in its placeholder (digit groups like
    // "1234 5678 9012 3456", not words like "Card number") is given them, unless its pattern is digits only.
    let groups = field.placeholder.split(' ').filter(|g| !g.is_empty() && g.chars().all(|c| c.is_ascii_digit() || matches!(c, '•' | '*' | 'x' | 'X' | '#'))).count();
    let spaces_allowed = field.pattern.is_empty() || field.pattern.contains(' ') || field.pattern.contains("\\s");
    let roomy = field.max_length.is_some_and(|m| m >= grouped.chars().count()) && groups >= 2 && spaces_allowed;
    let text = if roomy { grouped } else { digits };
    match field.max_length {
        Some(m) if text.chars().count() > m => Shaped::Left(format!("this field takes {m} characters and the card's number has {}", text.chars().count())),
        _ => shaped(text),
    }
}

/// The year and separator a field's `pattern` asks for, as (long year, no separator), when nothing else says: `\d{2}/\d{4}` is `MM/YYYY`, a bare
/// `\d{4}` is `MMYY` and a bare `\d{6}` is `MMYYYY`; anything else is the default `MM/YY`.
fn by_pattern(pattern: &str) -> (bool, bool) {
    let separated = pattern.contains('/') || pattern.contains("\\-") || pattern.contains(' ');
    let four = pattern.contains("{4}") && !pattern.contains("{2}{4}") && !pattern.contains(")?");
    match (four, pattern.contains("{6}"), separated) {
        (true, _, true) => (true, false),
        (true, _, false) => (false, true),
        (false, true, false) => (true, true),
        _ => (false, false),
    }
}

/// How a one-field expiry is written, from what the field says about itself.
fn expiry_text(field: &FormField, month: u32, year: i32) -> String {
    let placeholder = field.placeholder.to_ascii_uppercase();
    let pattern = field.pattern.as_str();
    // What stands between the month and year tokens of a placeholder like "MM / YY"; a placeholder
    // that is words ("Expiration date", "Exp. date") says nothing about the separator.
    let between = placeholder.find("MM").zip(placeholder.find("YY")).filter(|(m, y)| m + 2 <= *y).map(|(m, y)| &placeholder[m + 2..y]);
    let separator = between.and_then(|b| b.chars().find(|c| matches!(c, '/' | '-' | '.')).or_else(|| b.contains(' ').then_some(' ')));
    // The field's own length decides first (4 is MMYY, 5 MM/YY, 6 MMYYYY, 7 MM/YYYY); without one the
    // placeholder does ("MMYY" has no separator), and a pattern that asks for four digits.
    let (long, compact) = match field.max_length {
        Some(4) => (false, true),
        Some(5) => (false, false),
        Some(6) => (true, true),
        Some(7) => (true, false),
        _ if between.is_some() || placeholder.contains("YYYY") => (placeholder.contains("YYYY"), between.is_some() && separator.is_none()),
        _ => by_pattern(pattern),
    };
    let y = if long { format!("{year:04}") } else { format!("{:02}", year % 100) };
    let m = card::month_text(month);
    if compact {
        format!("{m}{y}")
    } else {
        format!("{m}{}{y}", separator.unwrap_or('/'))
    }
}

fn expiry(field: &FormField, parts: &Parts) -> Shaped {
    match card::expiry_of(parts) {
        Some((m, y)) => shaped(expiry_text(field, m, y)),
        None => left("expiry"),
    }
}

fn month(field: &FormField, parts: &Parts) -> Shaped {
    let Some((m, _)) = card::expiry_of(parts) else { return left("expiry") };
    let text = Zeroizing::new(card::month_text(m));
    if field.select {
        Shaped::Choose(text)
    } else {
        Shaped::Text(text)
    }
}

fn year(field: &FormField, parts: &Parts) -> Shaped {
    let Some((_, y)) = card::expiry_of(parts) else { return left("expiry") };
    if field.select {
        return Shaped::Choose(Zeroizing::new(format!("{y:04}")));
    }
    let short = field.max_length == Some(2) || field.placeholder.to_ascii_uppercase() == "YY";
    shaped(if short { format!("{:02}", y % 100) } else { format!("{y:04}") })
}

fn code(field: &FormField, parts: &Parts) -> Shaped {
    let Some(code) = parts.get("cvc").filter(|c| !c.is_empty()) else { return left("security code") };
    let length = code.chars().count();
    match field.max_length {
        Some(m) if m < length => Shaped::Left(format!("this field takes {m} digits and the card's code has {length}")),
        _ => shaped(code.to_string()),
    }
}

#[cfg(test)]
mod tests;
