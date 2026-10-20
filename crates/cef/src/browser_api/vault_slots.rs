// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which kind of Vault value a form field looks like
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.3), and
//! whether it is a password or one-time-code field, which the Vault never fills.
//!
//! Pure functions over `DOM.describeNode` facts, beside `secret.rs`, whose word
//! splitting they share. A field fitting a slot is a hint for srv's approval, not
//! a guarantee: the person still sees the site and the field.

use super::secret::{names_a_password, names_a_tax_id, words_of};
use super::snapshot::DomFacts;

/// The slots a field can fit, named as srv's `vault::slots` names a (kind, part).
pub const TAX_ID: &str = "tax-id";
pub const EIN: &str = "ein";
pub const DATE_OF_BIRTH: &str = "date-of-birth";
pub const PASSPORT: &str = "passport";
pub const DRIVERS_LICENCE: &str = "drivers-licence";
pub const CARD_NUMBER: &str = "card-number";
pub const CARD_EXPIRY: &str = "card-expiry";
pub const CARD_CVC: &str = "card-cvc";
pub const CARD_NAME: &str = "card-name";
pub const BANK_ACCOUNT: &str = "bank-account";
pub const BANK_ROUTING: &str = "bank-routing";
pub const API_KEY: &str = "api-key";

fn texts(f: &DomFacts) -> [String; 5] {
    [f.attr("name"), f.attr("id"), f.attr("aria-label"), f.attr("placeholder"), f.label.as_str()].map(str::to_string)
}

/// Every slot this field looks like (usually none or one).
pub fn slots(f: &DomFacts) -> Vec<&'static str> {
    let t = texts(f);
    let refs: Vec<&str> = t.iter().map(String::as_str).collect();
    let words: Vec<String> = refs.iter().flat_map(|s| words_of(s)).collect();
    let squashed: String = words.concat();
    let has = |w: &str| words.iter().any(|x| x == w);
    let contains = |s: &str| squashed.contains(s);
    let autocomplete = f.attr("autocomplete").to_ascii_lowercase();
    let tokens: Vec<&str> = autocomplete.split_whitespace().collect();
    let token = |s: &str| tokens.contains(&s);

    let mut out = Vec::new();
    if names_a_tax_id(&refs) {
        out.push(TAX_ID);
    }
    if has("ein") || contains("employeridentification") || contains("federaltaxid") {
        out.push(EIN);
    }
    if tokens.iter().any(|t| t.starts_with("bday")) || has("dob") || has("bday") || contains("dateofbirth") || contains("birthdate") || contains("birthday") {
        out.push(DATE_OF_BIRTH);
    }
    if contains("passport") {
        out.push(PASSPORT);
    }
    if contains("driverlicen") || contains("driverslicen") || contains("drivinglicen") {
        out.push(DRIVERS_LICENCE);
    }
    if token("cc-number") || contains("cardnumber") || contains("ccnumber") || contains("ccnum") || contains("creditcardnumber") {
        out.push(CARD_NUMBER);
    }
    if tokens.iter().any(|t| t.starts_with("cc-exp")) || contains("expir") || contains("ccexp") || contains("cardexp") {
        out.push(CARD_EXPIRY);
    }
    if token("cc-csc") || has("cvc") || has("cvv") || has("csc") || contains("securitycode") {
        out.push(CARD_CVC);
    }
    if token("cc-name") || contains("nameoncard") || contains("cardholder") || contains("ccname") {
        out.push(CARD_NAME);
    }
    if contains("accountnumber") || contains("bankaccount") || contains("acctnumber") || contains("accountno") || has("iban") {
        out.push(BANK_ACCOUNT);
    }
    if contains("routing") || has("aba") {
        out.push(BANK_ROUTING);
    }
    if has("token") || has("secret") || has("apikey") || contains("accesskey") || (has("key") && (has("api") || has("access") || has("secret"))) {
        out.push(API_KEY);
    }
    out
}

/// A password or one-time-code field: the Vault never fills these (spec §13
/// question 2), whatever the entry. Wider than `secret_reason`'s password cases:
/// it also takes an `autocomplete` that says password on a field named so.
pub fn password_like(f: &DomFacts) -> bool {
    if f.local_name == "input" && f.attr("type").eq_ignore_ascii_case("password") {
        return true;
    }
    let autocomplete = f.attr("autocomplete").to_ascii_lowercase();
    if autocomplete.split_whitespace().any(|t| t == "one-time-code") {
        return true;
    }
    let t = texts(f);
    let refs: Vec<&str> = t.iter().map(String::as_str).collect();
    if names_a_password(&refs) {
        return true;
    }
    let squashed: String = refs.iter().flat_map(|s| words_of(s)).collect::<Vec<_>>().concat();
    squashed.contains("onetimecode") || squashed.contains("onetimepassword") || words_of(&refs.join(" ")).iter().any(|w| w == "otp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn field(attrs: &[(&str, &str)], label: &str) -> DomFacts {
        DomFacts {
            local_name: "input".into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(),
            label: label.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_field_fits_the_kind_its_markup_or_label_names() {
        assert_eq!(slots(&field(&[("name", "ssn")], "")), [TAX_ID]);
        assert_eq!(slots(&field(&[("id", "taxpayerTIN")], "")), [TAX_ID]);
        assert_eq!(slots(&field(&[], "Social Security Number")), [TAX_ID]);
        assert_eq!(slots(&field(&[("name", "ein")], "")), [EIN]);
        assert_eq!(slots(&field(&[("autocomplete", "bday")], "")), [DATE_OF_BIRTH]);
        assert_eq!(slots(&field(&[("name", "dob")], "")), [DATE_OF_BIRTH]);
        assert_eq!(slots(&field(&[("name", "passportNumber")], "")), [PASSPORT]);
        assert_eq!(slots(&field(&[], "Driver's license no.")), [DRIVERS_LICENCE]);
        assert_eq!(slots(&field(&[("autocomplete", "cc-number")], "")), [CARD_NUMBER]);
        assert_eq!(slots(&field(&[("name", "card_number")], "")), [CARD_NUMBER]);
        assert_eq!(slots(&field(&[("autocomplete", "cc-exp")], "")), [CARD_EXPIRY]);
        assert_eq!(slots(&field(&[("name", "cvv")], "")), [CARD_CVC]);
        assert_eq!(slots(&field(&[("autocomplete", "cc-csc")], "")), [CARD_CVC]);
        assert_eq!(slots(&field(&[("autocomplete", "cc-name")], "")), [CARD_NAME]);
        assert_eq!(slots(&field(&[("name", "routingNumber")], "")), [BANK_ROUTING]);
        assert_eq!(slots(&field(&[("name", "account_number")], "")), [BANK_ACCOUNT]);
        assert_eq!(slots(&field(&[("name", "api_key")], "")), [API_KEY]);
        assert_eq!(slots(&field(&[("placeholder", "Secret token")], "")), [API_KEY]);
    }

    #[test]
    fn an_ordinary_field_fits_nothing() {
        for f in [
            field(&[("name", "city")], "City"),
            field(&[("name", "comment")], "Leave a comment"),
            field(&[("name", "email"), ("autocomplete", "new-password")], "Email"),
            field(&[("name", "monkey")], "Monkey"),
            field(&[("name", "keyword")], "Search"),
        ] {
            assert!(slots(&f).is_empty(), "{:?} fits {:?}", f.attrs, slots(&f));
        }
    }

    #[test]
    fn password_and_one_time_code_fields_are_never_filled() {
        assert!(password_like(&field(&[("type", "password")], "")));
        assert!(password_like(&field(&[("autocomplete", "one-time-code")], "")));
        assert!(password_like(&field(&[("name", "userPassword")], "")));
        assert!(password_like(&field(&[("name", "pin")], "")));
        assert!(password_like(&field(&[("name", "otp")], "")));
        assert!(password_like(&field(&[], "Enter your one-time code")));
        assert!(!password_like(&field(&[("name", "ssn")], "")));
        assert!(!password_like(&field(&[("autocomplete", "cc-number")], "")));
        assert!(!password_like(&field(&[("name", "email"), ("autocomplete", "new-password")], "Email")));
    }
}
