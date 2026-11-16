// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a card entry knows about itself: its brand, whether its number passes
//! the Luhn check, how its number is grouped, how long its security code is,
//! and its expiry month and year. Pure functions, no store, no seal.
//! docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §4.
//!
//! A card's number, expiry month and year, code, name and ZIP are sealed
//! parts. What the pane and the reminders can read without the key is the
//! plain `brand` and `default` fields and the `expires` date (the last day of
//! the expiry month), which [`settle`] keeps in step with the parts on every save.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use zeroize::Zeroize;

use super::seal::Parts;

/// The kind these functions are for.
pub const KIND: &str = "card";

/// The plain field that holds the brand (`visa`, `mastercard`, ...), worked out from the number.
pub const BRAND: &str = "brand";
/// The plain field that marks the person's default card (`yes`, or absent).
pub const DEFAULT: &str = "default";
/// A pre-split expiry, "MM/YY" or "MM/YYYY". Still read; a save writes `exp_month` and `exp_year` instead.
pub const LEGACY_EXPIRY: &str = "expiry";

/// The brand a number belongs to by its first digits, or an empty string.
pub fn brand(number: &str) -> &'static str {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    let prefix = |n: usize| -> u32 { digits.get(..n).and_then(|p| p.parse().ok()).unwrap_or(0) };
    let has = |n: usize| digits.len() >= n;
    if has(4) && (3528..=3589).contains(&prefix(4)) {
        "jcb"
    } else if has(2) && matches!(prefix(2), 34 | 37) {
        "amex"
    } else if (has(4) && prefix(4) == 6011) || (has(3) && (644..=649).contains(&prefix(3))) || (has(2) && prefix(2) == 65) {
        "discover"
    } else if (has(2) && (51..=55).contains(&prefix(2))) || (has(4) && (2221..=2720).contains(&prefix(4))) {
        "mastercard"
    } else if (has(3) && (300..=305).contains(&prefix(3))) || (has(2) && matches!(prefix(2), 36 | 38)) {
        "diners"
    } else if has(2) && prefix(2) == 62 {
        "unionpay"
    } else if digits.starts_with('4') {
        "visa"
    } else {
        ""
    }
}

/// A number's digits when it is only digits, spaces and dashes; None for anything else.
pub fn normalize_number(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

/// Whether the digits of `number` pass the Luhn check.
pub fn luhn(number: &str) -> bool {
    let digits: Vec<u32> = number.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() < 2 {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| if i % 2 == 1 { let x = d * 2; if x > 9 { x - 9 } else { x } } else { *d })
        .sum();
    sum % 10 == 0
}

/// A number grouped for reading: 4-6-5 for American Express, fours for the rest.
pub fn group(number: &str) -> String {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    let sizes: &[usize] = if brand(&digits) == "amex" { &[4, 6, 5] } else { &[4, 4, 4, 4, 4] };
    let mut out = String::new();
    let mut rest = digits.as_str();
    for size in sizes {
        if rest.is_empty() {
            break;
        }
        let (head, tail) = rest.split_at(rest.len().min(*size));
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(head);
        rest = tail;
    }
    if !rest.is_empty() {
        out.push(' ');
        out.push_str(rest);
    }
    out
}

/// How many digits the security code has: four for American Express, three otherwise.
pub fn cvc_length(brand: &str) -> usize {
    if brand == "amex" {
        4
    } else {
        3
    }
}

fn month_of(text: &str) -> Option<u32> {
    let m: u32 = text.trim().parse().ok()?;
    (1..=12).contains(&m).then_some(m)
}

fn year_of(text: &str) -> Option<i32> {
    let text = text.trim();
    let y: i32 = text.parse().ok()?;
    match text.len() {
        1 | 2 => Some(2000 + y),
        4 => (2000..=2099).contains(&y).then_some(y),
        _ => None,
    }
}

/// A month and year from any of "3/27", "03/27", "03/2027", "0327", "032027",
/// "03-2027" or "2027-03".
pub fn normalize_expiry(text: &str) -> Option<(u32, i32)> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let (a, b) = match text.split_once(['/', '-', '.']) {
        Some(pair) => pair,
        None if matches!(text.len(), 4 | 6) && text.is_ascii() => text.split_at(2),
        None => return None,
    };
    if !a.chars().all(|c| c.is_ascii_digit()) || !b.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if a.len() == 4 {
        return Some((month_of(b)?, year_of(a)?));
    }
    Some((month_of(a)?, year_of(b)?))
}

/// "03" for March.
pub fn month_text(month: u32) -> String {
    format!("{month:02}")
}

/// The last day of the expiry month: the card works through it.
pub fn last_day(month: u32, year: i32) -> Option<NaiveDate> {
    let first_of_next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) };
    first_of_next.and_then(|d| d.pred_opt())
}

/// Whether a card is past its month: from the first day of the month after.
pub fn expired(month: u32, year: i32, today: NaiveDate) -> bool {
    (today.year(), today.month()) > (year, month)
}

/// The expiry a set of parts holds: its month and year, or the older single
/// part read as one.
pub fn expiry_of(parts: &Parts) -> Option<(u32, i32)> {
    if let (Some(m), Some(y)) = (parts.get("exp_month").and_then(month_of), parts.get("exp_year").and_then(year_of)) {
        return Some((m, y));
    }
    parts.get(LEGACY_EXPIRY).and_then(normalize_expiry)
}

/// A card part as a fill or a command asks for it. The older `expiry` is read
/// from the month and year when the card has no such part, as "MM/YY".
pub fn part_value(parts: &Parts, part: &str) -> Option<String> {
    if let Some(v) = parts.get(part) {
        return Some(v.to_string());
    }
    if part == LEGACY_EXPIRY {
        let (m, y) = expiry_of(parts)?;
        return Some(format!("{}/{:02}", month_text(m), y % 100));
    }
    None
}

fn put(parts: &mut Parts, name: &str, value: String) {
    if let Some(mut old) = parts.0.insert(name.to_string(), value) {
        old.zeroize();
    }
}

/// Makes a card's parts and plain fields agree, on every save (by the person,
/// an agent or an import): the number as digits, the expiry as `exp_month` and
/// `exp_year` (the older `expiry` read into them and dropped), the brand worked
/// out from the number, `expires` set to the last day of the expiry month for
/// the reminders, and the default flag kept to `yes`. A month or year that is
/// not one is refused; a number with anything but digits is left as typed (the
/// pane warns, the Vault doesn't decide). Any other kind is left alone.
pub fn settle(kind: &str, fields: &mut BTreeMap<String, String>, parts: &mut Parts) -> Result<(), String> {
    if kind != KIND {
        return Ok(());
    }
    if let Some(digits) = parts.get("number").and_then(normalize_number) {
        if parts.get("number") != Some(digits.as_str()) {
            put(parts, "number", digits);
        }
    }
    let month = match parts.get("exp_month") {
        Some(m) => Some(month_of(m).ok_or("the expiry month is 1 to 12")?),
        None => None,
    };
    let year = match parts.get("exp_year") {
        Some(y) => Some(year_of(y).ok_or("the expiry year is four digits, like 2030, or two")?),
        None => None,
    };
    let older = if month.is_none() && year.is_none() { parts.get(LEGACY_EXPIRY).and_then(normalize_expiry) } else { None };
    let (month, year) = (month.or(older.map(|o| o.0)), year.or(older.map(|o| o.1)));
    if let Some(m) = month {
        put(parts, "exp_month", month_text(m));
    }
    if let Some(y) = year {
        put(parts, "exp_year", y.to_string());
    }
    fields.remove(super::expiry::FIELD);
    if let (Some(m), Some(y)) = (month, year) {
        if let Some(mut gone) = parts.0.remove(LEGACY_EXPIRY) {
            gone.zeroize();
        }
        if let Some(date) = last_day(m, y) {
            fields.insert(super::expiry::FIELD.to_string(), date.to_string());
        }
    }
    if let Some(zip) = parts.get("zip").map(str::trim).filter(|z| Some(*z) != parts.get("zip")).map(str::to_string) {
        put(parts, "zip", zip);
    }
    match parts.get("number").map(brand).filter(|b| !b.is_empty()) {
        Some(b) => {
            fields.insert(BRAND.to_string(), b.to_string());
        }
        None => {
            fields.remove(BRAND);
        }
    }
    if fields.get(DEFAULT).is_some_and(|v| v != "yes") {
        fields.remove(DEFAULT);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(list: &[(&str, &str)]) -> Parts {
        Parts(list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    #[test]
    fn brands_follow_the_first_digits() {
        for (number, want) in [
            ("4111111111111111", "visa"),
            ("5555555555554444", "mastercard"),
            ("2221000000000009", "mastercard"),
            ("2720999999999996", "mastercard"),
            ("378282246310005", "amex"),
            ("341111111111111", "amex"),
            ("6011111111111117", "discover"),
            ("6445644564456445", "discover"),
            ("6500000000000002", "discover"),
            ("30569309025904", "diners"),
            ("36227206271667", "diners"),
            ("38520000023237", "diners"),
            ("3530111333300000", "jcb"),
            ("3589000000000000", "jcb"),
            ("6200000000000005", "unionpay"),
            ("9999999999999999", ""),
            ("", ""),
            ("2", ""),
        ] {
            assert_eq!(brand(number), want, "{number}");
        }
        assert_eq!(brand("4111 1111 1111 1111"), "visa", "spaces are no matter");
    }

    #[test]
    fn luhn_accepts_the_test_numbers_and_refuses_one_digit_off() {
        for ok in ["4111111111111111", "5555555555554444", "378282246310005", "6011111111111117", "30569309025904", "3530111333300000"] {
            assert!(luhn(ok), "{ok}");
        }
        for bad in ["4111111111111112", "5555555555554445", "378282246310006", "1234567890123456", "7", ""] {
            assert!(!luhn(bad), "{bad}");
        }
        assert!(luhn("4111 1111 1111 1111"));
    }

    #[test]
    fn numbers_group_by_brand() {
        assert_eq!(group("4111111111111111"), "4111 1111 1111 1111");
        assert_eq!(group("378282246310005"), "3782 822463 10005");
        assert_eq!(group("4111"), "4111");
        assert_eq!(group("41111"), "4111 1");
        assert_eq!(group(""), "");
        assert_eq!(group("4111-1111-1111-1111"), "4111 1111 1111 1111");
    }

    #[test]
    fn a_number_is_digits_or_left_alone() {
        assert_eq!(normalize_number("4111 1111-1111 1111").as_deref(), Some("4111111111111111"));
        assert_eq!(normalize_number("4111x"), None);
        assert_eq!(normalize_number("  "), None);
    }

    #[test]
    fn the_code_is_four_digits_for_amex_only() {
        assert_eq!(cvc_length("amex"), 4);
        for b in ["visa", "mastercard", "discover", "diners", "jcb", "unionpay", ""] {
            assert_eq!(cvc_length(b), 3, "{b}");
        }
    }

    #[test]
    fn an_expiry_is_read_in_every_spelling() {
        for text in ["3/27", "03/27", "03/2027", "0327", "032027", "03-2027", "03 / 27", "3.27", "2027-03"] {
            assert_eq!(normalize_expiry(text), Some((3, 2027)), "{text}");
        }
        for text in ["13/27", "00/27", "03/1999", "soon", "", "3", "03/", "0327x", "03/20277"] {
            assert_eq!(normalize_expiry(text), None, "{text}");
        }
    }

    #[test]
    fn a_card_expires_after_the_last_day_of_its_month() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(last_day(2, 2028), Some(d(2028, 2, 29)));
        assert_eq!(last_day(12, 2027), Some(d(2027, 12, 31)));
        assert!(!expired(3, 2027, d(2027, 3, 31)));
        assert!(expired(3, 2027, d(2027, 4, 1)));
        assert!(expired(12, 2026, d(2027, 1, 1)));
        assert!(!expired(1, 2030, d(2026, 11, 10)));
    }

    #[test]
    fn the_older_expiry_is_read_as_a_month_and_year() {
        let old = parts(&[("number", "4111111111111111"), ("expiry", "03/2027")]);
        assert_eq!(expiry_of(&old), Some((3, 2027)));
        assert_eq!(part_value(&old, "expiry").as_deref(), Some("03/2027"));
        let new = parts(&[("exp_month", "03"), ("exp_year", "2027")]);
        assert_eq!(expiry_of(&new), Some((3, 2027)));
        assert_eq!(part_value(&new, "expiry").as_deref(), Some("03/27"), "the older part is derived as MM/YY");
        assert_eq!(part_value(&new, "cvc"), None);
        assert_eq!(expiry_of(&parts(&[("number", "4")])), None);
    }

    #[test]
    fn a_save_makes_the_parts_and_fields_agree() {
        let mut fields = BTreeMap::from([("brand".to_string(), "amex".to_string()), ("expires".to_string(), "1999-01-01".to_string())]);
        let mut p = parts(&[("number", "4111 1111 1111 1111"), ("exp_month", "3"), ("exp_year", "27"), ("zip", " 94107 ")]);
        settle("card", &mut fields, &mut p).unwrap();
        assert_eq!(p.get("number"), Some("4111111111111111"));
        assert_eq!((p.get("exp_month"), p.get("exp_year")), (Some("03"), Some("2027")));
        assert_eq!(p.get("zip"), Some("94107"));
        assert_eq!(fields.get("brand").map(String::as_str), Some("visa"), "the brand follows the number, not what was sent");
        assert_eq!(fields.get("expires").map(String::as_str), Some("2027-03-31"));
    }

    #[test]
    fn a_save_reads_the_older_expiry_into_the_new_parts_and_drops_it() {
        let mut fields = BTreeMap::new();
        let mut p = parts(&[("number", "378282246310005"), ("expiry", "11/2029"), ("cvc", "1234")]);
        settle("card", &mut fields, &mut p).unwrap();
        assert_eq!((p.get("exp_month"), p.get("exp_year"), p.get("expiry")), (Some("11"), Some("2029"), None));
        assert_eq!(fields.get("brand").map(String::as_str), Some("amex"));
        assert_eq!(fields.get("expires").map(String::as_str), Some("2029-11-30"));
        assert_eq!(p.get("cvc"), Some("1234"));
    }

    #[test]
    fn an_expiry_nobody_can_read_stays_as_it_was() {
        let mut fields = BTreeMap::from([("expires".to_string(), "2030-01-31".to_string())]);
        let mut p = parts(&[("number", "4111111111111111"), ("expiry", "someday")]);
        settle("card", &mut fields, &mut p).unwrap();
        assert_eq!(p.get("expiry"), Some("someday"));
        assert_eq!(p.get("exp_month"), None);
        assert!(!fields.contains_key("expires"), "a date the parts don't give is not kept");
    }

    #[test]
    fn a_bad_month_or_year_is_refused() {
        for (m, y) in [("13", "2030"), ("0", "2030"), ("x", "2030"), ("3", "1999"), ("3", "20300")] {
            let mut p = parts(&[("number", "4111111111111111"), ("exp_month", m), ("exp_year", y)]);
            assert!(settle("card", &mut BTreeMap::new(), &mut p).is_err(), "{m}/{y}");
        }
    }

    #[test]
    fn a_number_that_is_not_digits_is_kept_and_has_no_brand() {
        let mut fields = BTreeMap::from([("brand".to_string(), "visa".to_string())]);
        let mut p = parts(&[("number", "GIFT-ABCD-1234")]);
        settle("card", &mut fields, &mut p).unwrap();
        assert_eq!(p.get("number"), Some("GIFT-ABCD-1234"));
        assert!(!fields.contains_key("brand"));
    }

    #[test]
    fn only_yes_marks_the_default_and_other_kinds_are_left_alone() {
        let mut fields = BTreeMap::from([("default".to_string(), "true".to_string())]);
        settle("card", &mut fields, &mut parts(&[("number", "4111111111111111")])).unwrap();
        assert!(!fields.contains_key("default"));
        let mut fields = BTreeMap::from([("default".to_string(), "yes".to_string())]);
        settle("card", &mut fields, &mut parts(&[("number", "4111111111111111")])).unwrap();
        assert_eq!(fields.get("default").map(String::as_str), Some("yes"));
        let mut other = BTreeMap::from([("expires".to_string(), "2030-01-01".to_string())]);
        settle("api-key", &mut other, &mut parts(&[("value", "k")])).unwrap();
        assert_eq!(other.len(), 1);
    }
}
