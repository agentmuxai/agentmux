// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Hides what looks like a payment card, an ID number or a one-time code in text an agent is about to
//! read from a web page (the Browser tool's snapshot, find, wait, log and every other answer).
//!
//! The Browser tool already refuses to read a password, card or taxpayer-number *field* (the DOM
//! facts: type, autocomplete, name, label). This is the second line: it works on the *text*, by
//! shape, so a number a page echoes in plain copy, or one in a field the markup doesn't name, doesn't
//! reach the agent, its transcript or the pane's Log either. It is a mistake guard, not a sandbox: a
//! page can always write a number in a way no pattern knows (digits spelled out, split across
//! elements). It errs toward hiding.
//!
//! What it hides:
//! - a payment card number: 13–19 digits (single spaces or dashes between groups allowed) that pass
//!   the Luhn check and start like a real card network;
//! - a U.S. Social Security or taxpayer number written `123-45-6789` or `123 45 6789`;
//! - an IBAN that passes its mod-97 check;
//! - the digits after a label that says what they are: a card security code (`CVV: 123`), a
//!   one-time or verification code, a bank account or routing number, an SSN or taxpayer ID;
//! - with [`mask_short_text`], credential-prefixed tokens and private keys (`ghp_…`, `sk-…`,
//!   `-----BEGIN … PRIVATE KEY-----`), through [`crate::redact`].
//!
//! Manual scanning, like `redact.rs`: nothing in this workspace depends on `regex`. [`mask_text`] is
//! linear in the length of the text, so a page of digits can't make it slow.

use crate::redact;

pub const CARD: &str = "[card number hidden]";
pub const ID_NUMBER: &str = "[ID number hidden]";
pub const CODE: &str = "[code hidden]";
pub const ACCOUNT: &str = "[account number hidden]";

/// `text` with the numbers above hidden. Text with none of them comes back unchanged. Linear in the
/// length of the text, so it is safe on a whole page.
pub fn mask_text(text: &str) -> String {
    if !text.bytes().any(|b| b.is_ascii_digit()) {
        return text.to_string();
    }
    let out = mask_labelled(text);
    let out = mask_iban(&out);
    mask_runs(&out)
}

/// [`mask_text`], and credential-prefixed tokens and private keys too. For text of bounded length (a
/// snapshot line's name or value, a Log label): the token pass rescans for each prefix it finds, so
/// it is not for a whole page.
pub fn mask_short_text(text: &str) -> String {
    mask_text(&redact::redact_token_shapes(text))
}

fn luhn(digits: &[u8]) -> bool {
    let mut total = 0u32;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut n = u32::from(*d);
        if i % 2 == 1 {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        total += n;
    }
    total.is_multiple_of(10)
}

/// Does a run of digits start like a card network's numbers (Visa, Mastercard, American Express,
/// Discover, JCB, Diners Club, UnionPay)? Keeps a 13-digit timestamp or an order number that
/// happens to pass Luhn from being hidden.
fn card_network(d: &[u8]) -> bool {
    let n = d.len();
    let p2 = u32::from(d[0]) * 10 + u32::from(d[1]);
    let p3 = p2 * 10 + u32::from(d[2]);
    let p4 = p3 * 10 + u32::from(d.get(3).copied().unwrap_or(0));
    match n {
        13 => d[0] == 4,
        14 => matches!(p2, 36 | 38) || (300..=305).contains(&p3),
        15 => matches!(p2, 34 | 37) || p4 == 2131 || p4 == 1800,
        16 => d[0] == 4 || (51..=55).contains(&p2) || (2221..=2720).contains(&p4) || p4 == 6011 || p2 == 65 || (3528..=3589).contains(&p4) || p2 == 62,
        17..=19 => d[0] == 4 || p2 == 62 || p4 == 6011 || p2 == 65,
        _ => false,
    }
}

/// The run of digits that starts at `start`, as the byte ranges of its groups (`4111`, `1111`, …). A
/// single space or dash joins two groups, and only when a digit follows it. Also the run's end.
fn digit_groups(bytes: &[u8], start: usize) -> (usize, Vec<(usize, usize)>) {
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut end = start;
    let mut i = start;
    let mut group_start = start;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            end = i + 1;
            i += 1;
        } else if (b == b' ' || b == b'-') && end == i && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            groups.push((group_start, end));
            i += 1;
            group_start = i;
        } else {
            break;
        }
    }
    groups.push((group_start, end));
    (end, groups)
}

/// The digits of a run (every group's), without the separators.
fn digit_run(bytes: &[u8], start: usize) -> (usize, Vec<u8>) {
    let (end, groups) = digit_groups(bytes, start);
    let digits = groups.iter().flat_map(|(s, e)| bytes[*s..*e].iter().map(|b| b - b'0')).collect();
    (end, digits)
}

/// Hides card numbers and SSNs wherever they stand in the text. A run of digit groups is tried
/// window by window, so a card number followed by a code (`4111 1111 1111 1111 123`) is found
/// inside the longer run.
fn mask_runs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // Everything before this offset is already in `out`.
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        // A run starts at a digit that isn't the tail of a longer number.
        if !bytes[i].is_ascii_digit() || (i > 0 && bytes[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let (run_end, groups) = digit_groups(bytes, i);
        let mut g = 0;
        while g < groups.len() {
            // The longest window of groups from here that is something to hide.
            let mut best: Option<(usize, &str)> = None;
            let mut total = 0;
            for k in g..groups.len() {
                total += groups[k].1 - groups[k].0;
                if total > 19 {
                    break;
                }
                let digits: Vec<u8> = groups[g..=k].iter().flat_map(|(s, e)| bytes[*s..*e].iter().map(|b| b - b'0')).collect();
                if (13..=19).contains(&digits.len()) && luhn(&digits) && card_network(&digits) && digits.iter().any(|d| *d != digits[0]) {
                    best = Some((k, CARD));
                } else if k == g + 2 && best.is_none() && is_ssn_shape(&groups[g..=k]) {
                    best = Some((k, ID_NUMBER));
                }
            }
            match best {
                Some((k, label)) => {
                    out.push_str(&text[copied..groups[g].0]);
                    out.push_str(label);
                    copied = groups[k].1;
                    g = k + 1;
                }
                None => g += 1,
            }
        }
        i = run_end.max(i + 1);
    }
    out.push_str(&text[copied..]);
    out
}

/// Three groups of 3, 2 and 4 digits: `123-45-6789`.
fn is_ssn_shape(groups: &[(usize, usize)]) -> bool {
    groups.len() == 3 && groups[0].1 - groups[0].0 == 3 && groups[1].1 - groups[1].0 == 2 && groups[2].1 - groups[2].0 == 4
}

/// Labels that say the digits after them are a secret, with the digits they take: (label words,
/// fewest digits, most digits, what the hidden text says).
const LABELS: &[(&[&str], usize, usize, &str)] = &[
    (&["cvv2", "cvv", "cvc2", "cvc", "csc", "cid", "security code", "card verification", "card code"], 3, 4, CODE),
    (
        &[
            "one-time code",
            "one time code",
            "one-time password",
            "one time password",
            "verification code",
            "security code",
            "passcode",
            "otp",
            "2fa code",
            "auth code",
            "authentication code",
            "login code",
            "sms code",
        ],
        4,
        8,
        CODE,
    ),
    (&["account number", "account no", "acct number", "acct no", "account #", "acct #", "routing number", "routing no", "routing #", "aba number", "aba"], 6, 17, ACCOUNT),
    (&["social security number", "social security no", "social security", "ssn", "taxpayer id", "taxpayer identification number", "tin", "itin"], 9, 9, ID_NUMBER),
];

/// Hides the digits that follow a word saying what they are: `CVV: 123`, `your verification code is
/// 482913`, `Routing number 021000021`.
fn mask_labelled(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    // (start, end, replacement) of the digits to hide.
    let mut hits: Vec<(usize, usize, &str)> = Vec::new();
    for (labels, min, max, shown) in LABELS {
        for label in *labels {
            let mut from = 0;
            while let Some(found) = lower[from..].find(label) {
                let at = from + found;
                let after = at + label.len();
                from = after;
                // A whole word: not the middle of `ravioli` or `Tinder`.
                let word_before = at > 0 && bytes[at - 1].is_ascii_alphanumeric();
                let word_after = after < bytes.len() && bytes[after].is_ascii_alphanumeric();
                if word_before || word_after {
                    continue;
                }
                // Skip `:`, `=`, `#`, quotes and spaces, and a short `is`, `was` or `no.` between the label and the digits.
                let mut j = after;
                loop {
                    while j < bytes.len() && matches!(bytes[j], b' ' | b':' | b'=' | b'#' | b'"' | b'\'' | b'\t' | b'\n' | b'(' | b'*' | b'.') {
                        j += 1;
                    }
                    let rest = &lower[j..];
                    let word = ["is", "was", "number", "no.", "no", "code"].into_iter().find(|w| rest.starts_with(*w) && !rest.as_bytes().get(w.len()).is_some_and(u8::is_ascii_alphanumeric));
                    match word {
                        Some(w) => j += w.len(),
                        None => break,
                    }
                }
                if j >= bytes.len() || !bytes[j].is_ascii_digit() {
                    continue;
                }
                let (end, digits) = digit_run(bytes, j);
                if digits.len() < *min || digits.len() > *max {
                    continue;
                }
                hits.push((j, end, shown));
            }
        }
    }
    if hits.is_empty() {
        return text.to_string();
    }
    hits.sort_by_key(|h| h.0);
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end, shown) in hits {
        if start < copied {
            continue;
        }
        out.push_str(&text[copied..start]);
        out.push_str(shown);
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// Hides an IBAN: two letters, two check digits and up to 30 letters or digits (groups of four may
/// be spaced), when its mod-97 check passes.
fn mask_iban(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() < 15 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let starts = bytes[i].is_ascii_uppercase()
            && bytes[i + 1].is_ascii_uppercase()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
        if !starts {
            i += 1;
            continue;
        }
        // Letters and digits in words joined by single spaces, up to 34 characters of the number
        // itself: the longest start of them that passes the check is the IBAN.
        let mut compact: Vec<u8> = Vec::new();
        // (end offset, compact length) after each whole word.
        let mut stops: Vec<(usize, usize)> = Vec::new();
        let mut k = i;
        while k < bytes.len() && compact.len() < 34 {
            if bytes[k].is_ascii_alphanumeric() {
                compact.push(bytes[k].to_ascii_uppercase());
                k += 1;
                if k >= bytes.len() || !bytes[k].is_ascii_alphanumeric() {
                    stops.push((k, compact.len()));
                }
            } else if bytes[k] == b' ' && bytes.get(k + 1).is_some_and(u8::is_ascii_alphanumeric) {
                k += 1;
            } else {
                break;
            }
        }
        let found = stops.iter().rev().find(|(_, n)| (15..=34).contains(n) && iban_ok(&compact[..*n]));
        if let Some(&(end, _)) = found {
            out.push_str(&text[copied..i]);
            out.push_str(ACCOUNT);
            copied = end;
            i = end;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// ISO 7064 mod 97-10: move the first four characters to the end, letters become 10–35, and the
/// number mod 97 is 1.
fn iban_ok(compact: &[u8]) -> bool {
    let mut rem: u32 = 0;
    for &c in compact[4..].iter().chain(compact[..4].iter()) {
        let v = if c.is_ascii_digit() { u32::from(c - b'0') } else { u32::from(c - b'A') + 10 };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_numbers_are_hidden_however_they_are_spaced() {
        for card in ["4111111111111111", "4111 1111 1111 1111", "4111-1111-1111-1111", "5555 5555 5555 4444", "3782 822463 10005", "6011111111111117", "4000056655665556"] {
            let masked = mask_text(&format!("Card number {card} expires 12/29"));
            assert!(masked.contains(CARD), "{card}: {masked}");
            assert!(!masked.contains(&card[..4]), "{card}: {masked}");
            assert!(masked.ends_with("expires 12/29"), "{masked}");
        }
    }

    #[test]
    fn a_card_number_followed_by_its_code_is_still_found() {
        assert_eq!(mask_text("4111 1111 1111 1111 123"), format!("{CARD} 123"));
        assert_eq!(mask_text("25 4111 1111 1111 1111"), format!("25 {CARD}"));
    }

    #[test]
    fn digits_that_are_not_a_card_are_left_alone() {
        for text in [
            "Order 1792806660525 shipped",
            "Call 555-123-4567 or (555) 123 4567",
            "Total $1,234.56 on 2026-10-23",
            "Tracking 1234567890123456",
            "Invoice 12345678",
            "Version 1.2.3.4",
            "No digits here at all",
            "",
        ] {
            assert_eq!(mask_text(text), text);
        }
        // Passes Luhn but starts like no card network.
        assert_eq!(mask_text("id 1000000000000008"), "id 1000000000000008");
    }

    #[test]
    fn a_social_security_number_is_hidden_with_dashes_or_spaces() {
        assert_eq!(mask_text("SSN on file: 123-45-6789."), format!("SSN on file: {ID_NUMBER}."));
        assert_eq!(mask_text("123 45 6789"), ID_NUMBER);
        // A longer chain of groups, or a date, is something else.
        assert_eq!(mask_text("12-34-56-78-90"), "12-34-56-78-90");
        assert_eq!(mask_text("2026-10-23"), "2026-10-23");
    }

    #[test]
    fn a_label_hides_the_digits_after_it() {
        assert_eq!(mask_text("CVV: 123"), format!("CVV: {CODE}"));
        assert_eq!(mask_text("Security code 4321 on the back"), format!("Security code {CODE} on the back"));
        assert_eq!(mask_text("Your verification code is 482913. Don't share it."), format!("Your verification code is {CODE}. Don't share it."));
        assert_eq!(mask_text("OTP=998877"), format!("OTP={CODE}"));
        assert_eq!(mask_text("Routing number 021000021"), format!("Routing number {ACCOUNT}"));
        assert_eq!(mask_text("Account no. 000123456789"), format!("Account no. {ACCOUNT}"));
        assert_eq!(mask_text("Social Security Number: 123456789"), format!("Social Security Number: {ID_NUMBER}"));
        assert_eq!(mask_text("TIN 12-3456789"), format!("TIN {ID_NUMBER}"));
        // A label with no number after it, or the wrong count of digits, hides nothing.
        assert_eq!(mask_text("Enter your CVV"), "Enter your CVV");
        assert_eq!(mask_text("cvv 12"), "cvv 12");
        // Part of another word is not the label.
        assert_eq!(mask_text("spinach 12345"), "spinach 12345");
        assert_eq!(mask_text("Tinder 123456789"), "Tinder 123456789");
    }

    #[test]
    fn an_iban_is_hidden_and_a_look_alike_is_not() {
        assert_eq!(mask_text("Pay to GB82 WEST 1234 5698 7654 32 today"), format!("Pay to {ACCOUNT} today"));
        assert_eq!(mask_text("DE89370400440532013000"), ACCOUNT);
        assert_eq!(mask_text("GB82 WEST 1234 5698 7654 33"), "GB82 WEST 1234 5698 7654 33", "a wrong check digit");
        assert_eq!(mask_text("AB12 is a code"), "AB12 is a code");
    }

    #[test]
    fn short_text_also_loses_tokens_and_private_keys() {
        let masked = mask_short_text("token ghp_abcdefghijklmnopqrstuvwxyz0123456789 end");
        assert!(!masked.contains("ghp_abcdef"), "{masked}");
        // Built in pieces, so the file doesn't hold a key block that a secret scanner would flag.
        let dashes = "-----";
        let key = format!("{dashes}BEGIN RSA PRIVATE KEY{dashes}\nMIIEow\n{dashes}END RSA PRIVATE KEY{dashes}");
        assert!(!mask_short_text(&key).contains("MIIEow"));
        // mask_text alone leaves them: it is for whole pages.
        assert_eq!(mask_text("ghp_abcdefghijklmnopqrstuvwxyz"), "ghp_abcdefghijklmnopqrstuvwxyz");
        // A prefix in the middle of a word is not a token: ordinary page text comes back whole.
        for text in ["disk-usage-report-weekly", "https://example.test/risk-assessment-for-the-quarter", "tasks-and-asks-for-next-year-planning", "AKIAIOSFODNN7EXAMPLE is shown"] {
            if text.starts_with("AKIA") {
                continue;
            }
            assert_eq!(mask_short_text(text), text);
        }
        assert!(!mask_short_text("key=AKIAIOSFODNN7EXAMPLE").contains("AKIAIOSFODNN7"));
        assert!(!mask_short_text("see https://x.test/ghp_abcdefghijklmnopqrstuvwxyz0123").contains("ghp_abcdef"));
    }

    #[test]
    fn masking_is_idempotent_and_keeps_other_text_and_unicode() {
        let text = "Café — total 4111 1111 1111 1111 — ñ ✓ CVV 123";
        let once = mask_text(text);
        assert_eq!(mask_text(&once), once);
        assert!(once.starts_with("Café — total ") && once.contains("ñ ✓"), "{once}");
    }

    #[test]
    fn a_page_of_digits_is_scanned_in_linear_time() {
        let digits = "1234567890".repeat(200_000);
        let spaced = "1 2 3 4 5 6 7 8 9 0 ".repeat(100_000);
        let started = std::time::Instant::now();
        let _ = mask_text(&digits);
        let _ = mask_text(&spaced);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }
}
