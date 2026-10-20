// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The secret-field rule: which form fields an agent must never type into or
//! read (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.1), and
//! why, in words the snapshot mark and the refusal show. Pure functions over
//! `DOM.describeNode` facts, split out of `snapshot.rs`.

use super::snapshot::DomFacts;

/// Split a field name or label into lower-case words at spaces, punctuation,
/// camelCase and letter/digit boundaries: `taxpayerTIN` → `taxpayer`, `tin`;
/// `SSNInput` → `ssn`, `input`; `last4ssn` → `last`, `4`, `ssn`.
pub(super) fn words_of(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur).to_lowercase());
            }
            continue;
        }
        if let Some(prev) = cur.chars().last() {
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (prev.is_lowercase() && c.is_uppercase())
                || (prev.is_uppercase() && c.is_uppercase() && next_is_lower)
                || (prev.is_alphabetic() != c.is_alphabetic());
            if boundary {
                words.push(std::mem::take(&mut cur).to_lowercase());
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur.to_lowercase());
    }
    words
}

/// Does any of these names a person's taxpayer identification number: an SSN,
/// ITIN or TIN? A guard for mistakes and page tricks, so a false alarm (a
/// field about tin cans) only means the user fills it. A company's EIN isn't
/// one: it is printed on public filings.
pub(super) fn names_a_tax_id(texts: &[&str]) -> bool {
    texts.iter().any(|t| {
        let words = words_of(t);
        let squashed: String = words.concat();
        words.iter().any(|w| w == "ssn" || w == "itin" || w == "tin")
            || squashed.contains("socialsecurity")
            || squashed.contains("taxpayerid")
    })
}

/// Does any of these name a password: password, passphrase, passcode, PIN, pw?
pub(super) fn names_a_password(texts: &[&str]) -> bool {
    texts.iter().any(|t| {
        words_of(t).iter().any(|w| {
            w.contains("password") || w.contains("passwd") || w.contains("passphrase") || w.contains("passcode")
                || w == "pin" || w == "pw" || w == "pwd"
        })
    })
}

/// A field the agent must never type into or read: passwords, one-time codes,
/// card details and taxpayer identification numbers (spec §5.1). A guard against mistakes and page tricks,
/// not a sandbox.
pub fn is_secret(f: &DomFacts) -> bool {
    secret_reason(f).is_some()
}

/// Why [`is_secret`] holds, in words for the agent and the person: the
/// snapshot's mark and the refusal name it, so a false alarm is easy to see.
pub fn secret_reason(f: &DomFacts) -> Option<&'static str> {
    // A field the Vault filled holds a value no agent may read back, whatever its name.
    if f.attrs.contains_key(super::vault_fill::TAINT_ATTR) {
        return Some("it holds a value filled from the Vault");
    }
    if f.local_name == "input" && f.attr("type").eq_ignore_ascii_case("password") {
        return Some("it is a password field");
    }
    let autocomplete = f.attr("autocomplete").to_ascii_lowercase();
    let tokens: Vec<&str> = autocomplete.split_whitespace().collect();
    if tokens.contains(&"one-time-code") {
        return Some("its autocomplete says one-time-code");
    }
    if tokens.iter().any(|t| t.starts_with("cc-")) {
        return Some("its autocomplete says it is a card field");
    }
    // Sites put autocomplete="new-password" on ordinary fields (an address, an
    // email) because browsers ignore "off": it counts only on a field that is
    // also called a password (a "show password" button makes one type=text).
    if tokens.iter().any(|t| *t == "current-password" || *t == "new-password")
        && names_a_password(&[f.attr("name"), f.attr("id"), f.attr("aria-label"), f.attr("placeholder"), &f.label])
    {
        return Some("its autocomplete and its name say password");
    }
    // A person's tax identification number: named by the markup or by what a
    // person reads next to the field.
    if names_a_tax_id(&[f.attr("name"), f.attr("id"), f.attr("aria-label"), f.attr("placeholder"), &f.label]) {
        return Some("its name or label says Social Security or taxpayer ID number");
    }
    named_like_a_secret(f).then_some("its name says password, one-time code, card number or security code")
}

/// Named like one: password, passcode, otp, one-time code, cvc/cvv, card number.
fn named_like_a_secret(f: &DomFacts) -> bool {
    let names = [f.attr("name"), f.attr("id")].join(" ").to_ascii_lowercase();
    let squashed: String = names.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
    squashed.split_whitespace().any(|w| {
        w.contains("password")
            || w.contains("passwd")
            || w.contains("passcode")
            || w == "otp"
            || w.starts_with("otp")
            || w.contains("onetimecode")
            || w.contains("onetimepassword")
            // card_cvc, payment-cvv and card[cvc] squash to one word.
            || w.contains("cvc")
            || w.contains("cvv")
            || w.contains("cardnumber")
            || w.contains("ccnumber")
            || w.contains("securitycode")
    }) || names.contains("one-time") || names.contains("one_time") || names.contains("card-number") || names.contains("card_number")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_fields_are_recognized_by_type_autocomplete_and_name() {
        let f = |tag: &str, attrs: &[(&str, &str)]| DomFacts {
            local_name: tag.into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            label: String::new(),
        };
        assert!(is_secret(&f("input", &[("type", "password")])));
        assert!(is_secret(&f("input", &[("autocomplete", "one-time-code")])));
        assert!(is_secret(&f("input", &[("autocomplete", "section-pay cc-number")])));
        assert!(is_secret(&f("input", &[("name", "user_password")])));
        assert!(is_secret(&f("input", &[("id", "otp-input")])));
        assert!(is_secret(&f("input", &[("name", "card-number")])));
        assert!(is_secret(&f("input", &[("name", "cvc")])));
        assert!(is_secret(&f("input", &[("name", "card_cvc")])));
        assert!(is_secret(&f("input", &[("name", "payment-cvv")])));
        assert!(is_secret(&f("input", &[("name", "card[cvc]")])));
        assert!(!is_secret(&f("input", &[("type", "email"), ("name", "email")])));
        assert!(!is_secret(&f("input", &[("name", "detection_name"), ("autocomplete", "off")])));
        assert!(!is_secret(&f("textarea", &[("name", "comments")])));
    }

    #[test]
    fn autocomplete_new_password_alone_does_not_make_an_ordinary_field_secret() {
        // A state registry's submitter form puts autocomplete="new-password"
        // on its address, city, ZIP and email fields to stop browser autofill.
        let f = |attrs: &[(&str, &str)], label: &str| DomFacts {
            local_name: "input".into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            label: label.into(),
        };
        for name in ["mailingAddress1", "city", "postalCode", "email"] {
            let field = f(&[("type", "text"), ("autocomplete", "new-password"), ("formcontrolname", name)], "");
            assert!(!is_secret(&field), "{name}");
        }
        assert!(!is_secret(&f(&[("autocomplete", "current-password"), ("name", "username")], "Username")));
        assert!(!is_secret(&f(&[("autocomplete", "new-password"), ("name", "spinner")], "")));
        // A real password field stays secret, whatever type a show-password
        // button gave it: by its type, its name or its label.
        assert_eq!(secret_reason(&f(&[("type", "password"), ("autocomplete", "new-password")], "")), Some("it is a password field"));
        assert!(is_secret(&f(&[("type", "text"), ("autocomplete", "new-password"), ("name", "newPassword")], "")));
        assert_eq!(
            secret_reason(&f(&[("type", "text"), ("autocomplete", "current-password"), ("name", "field_7")], "Password")),
            Some("its autocomplete and its name say password")
        );
        assert!(is_secret(&f(&[("autocomplete", "new-password"), ("name", "pin")], "")));
        assert!(is_secret(&f(&[("autocomplete", "new-password"), ("id", "pw")], "")));
        assert!(is_secret(&f(&[("autocomplete", "current-password")], "Passphrase")));
        // One-time codes and card fields need no name: sites don't use those
        // tokens to block autofill.
        assert_eq!(secret_reason(&f(&[("autocomplete", "one-time-code")], "")), Some("its autocomplete says one-time-code"));
        assert_eq!(secret_reason(&f(&[("autocomplete", "cc-exp")], "")), Some("its autocomplete says it is a card field"));
    }

    #[test]
    fn taxpayer_identification_numbers_are_secret_fields() {
        let f = |attrs: &[(&str, &str)], label: &str| DomFacts {
            local_name: "input".into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            label: label.into(),
        };
        // By what a person reads next to the field, by markup, in any casing.
        assert!(is_secret(&f(&[("type", "text")], "Taxpayer's TIN")));
        assert!(is_secret(&f(&[], "Social Security Number (SSN)")));
        assert!(is_secret(&f(&[], "Taxpayer identification number")));
        assert!(is_secret(&f(&[("name", "taxpayerTIN")], "")));
        assert!(is_secret(&f(&[("id", "payer_tin")], "")));
        assert!(is_secret(&f(&[("name", "SSNInput")], "")));
        assert!(is_secret(&f(&[("name", "last4ssn")], "")));
        assert!(is_secret(&f(&[("name", "ssn-1")], "")));
        assert!(is_secret(&f(&[("aria-label", "ITIN")], "")));
        assert!(is_secret(&f(&[("placeholder", "Enter your SSN")], "")));
        assert!(is_secret(&f(&[("name", "socialSecurityNumber")], "")));
        assert!(is_secret(&f(&[("name", "taxpayer-id")], "")));
        // Words that merely contain the letters, and a company's EIN, are not.
        assert!(!is_secret(&f(&[("name", "setting")], "Settings")));
        assert!(!is_secret(&f(&[("name", "routing")], "Routing preference")));
        assert!(!is_secret(&f(&[("name", "destination")], "Destination")));
        assert!(!is_secret(&f(&[("name", "ein")], "Employer identification number (EIN)")));
        assert!(!is_secret(&f(&[("name", "street")], "Street address")));
    }

    #[test]
    fn words_split_at_case_digit_and_punctuation_boundaries() {
        assert_eq!(words_of("taxpayerTIN"), ["taxpayer", "tin"]);
        assert_eq!(words_of("SSNInput"), ["ssn", "input"]);
        assert_eq!(words_of("last4ssn"), ["last", "4", "ssn"]);
        assert_eq!(words_of("Taxpayer's TIN"), ["taxpayer", "s", "tin"]);
        assert_eq!(words_of("card[cvc]"), ["card", "cvc"]);
        assert!(words_of("").is_empty());
    }
}
