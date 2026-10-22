// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The names a Vault entry's parts and details have in a KeePass entry
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.3, §8.4).
//!
//! KeePass has an entry type of one kind: a title, a user name, a password, an address,
//! notes, and any number of named custom fields. So a card, a Social Security number or an
//! API key is an entry whose secret is a *protected custom field* with a name. The exporter
//! writes these names and the importer reads the same ones (and a few spellings other
//! people use), so a database leaves and comes back as the same entries.

/// A secret custom field that makes an entry a Vault entry of `kind`, holding its `part`.
pub struct Primary {
    pub kind: &'static str,
    pub part: &'static str,
    /// What the exporter calls the field.
    pub title: &'static str,
    /// Every name the importer takes for it, lowercase.
    pub aliases: &'static [&'static str],
}

pub const PRIMARY: &[Primary] = &[
    Primary { kind: "ssn", part: "value", title: "SSN", aliases: &["ssn", "social security number"] },
    Primary { kind: "itin", part: "value", title: "ITIN", aliases: &["itin"] },
    Primary { kind: "tin", part: "value", title: "Taxpayer ID", aliases: &["taxpayer id", "taxpayer id number", "tin"] },
    Primary { kind: "ein", part: "value", title: "EIN", aliases: &["ein"] },
    Primary { kind: "date-of-birth", part: "value", title: "Date of birth", aliases: &["date of birth", "dob"] },
    Primary { kind: "passport", part: "value", title: "Passport number", aliases: &["passport number", "passport"] },
    Primary {
        kind: "drivers-licence",
        part: "value",
        title: "Driver's licence number",
        aliases: &["driver's licence number", "driver's license number", "drivers licence number", "drivers license number"],
    },
    Primary { kind: "other-id", part: "value", title: "ID number", aliases: &["id number"] },
    Primary { kind: "card", part: "number", title: "Card number", aliases: &["card number", "credit card number"] },
    Primary { kind: "bank-account", part: "account", title: "Account number", aliases: &["account number", "bank account number"] },
    Primary { kind: "api-key", part: "value", title: "API key", aliases: &["api key", "api_key", "apikey"] },
];

/// A secret that goes with a primary one: a card's expiry, a bank account's routing number.
pub struct Companion {
    pub kind: &'static str,
    pub part: &'static str,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
}

pub const COMPANIONS: &[Companion] = &[
    Companion { kind: "card", part: "expiry", title: "Expiry", aliases: &["expiry", "expiry date", "expires"] },
    Companion { kind: "card", part: "cvc", title: "CVC", aliases: &["cvc", "cvv", "security code"] },
    Companion { kind: "card", part: "name", title: "Cardholder", aliases: &["cardholder", "cardholder name", "name on card"] },
    Companion { kind: "bank-account", part: "routing", title: "Routing number", aliases: &["routing number", "routing"] },
];

/// A plain detail: not secret, so an unprotected custom field.
pub struct Detail {
    pub name: &'static str,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
}

pub const DETAILS: &[Detail] = &[
    Detail { name: "owner", title: "Owner", aliases: &["owner"] },
    Detail { name: "issuer", title: "Issuer", aliases: &["issuer"] },
    Detail { name: "expires", title: "Expires", aliases: &["expires on", "valid until", "expiration date"] },
    Detail { name: "service", title: "Service", aliases: &["service"] },
    Detail { name: "env_name", title: "Environment variable", aliases: &["environment variable", "env name", "env_name"] },
    Detail { name: "bank", title: "Bank", aliases: &["bank", "bank name"] },
];

pub fn primary_for(kind: &str) -> Option<&'static Primary> {
    PRIMARY.iter().find(|p| p.kind == kind)
}

pub fn matches(aliases: &[&str], name: &str) -> bool {
    let n = name.trim().to_lowercase();
    aliases.iter().any(|a| *a == n)
}
