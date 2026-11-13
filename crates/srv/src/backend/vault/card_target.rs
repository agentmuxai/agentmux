// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The card a whole-card fill is for, resolved by name before anything is asked
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! `Browser` `fill` with `vault: "<card id>"` names the card; `vault: "card"` means the
//! person's default one. `vault: "<id>.<part>"` is the older one-field fill and is not
//! resolved here. What is returned is what the person is shown in the approval (the
//! brand, the last four and the expiry) and nothing the agent could use.

use super::{card, kinds, seal, vaults, FillTarget};
use crate::backend::storage::store::Store;

/// A card named for a whole-card fill.
pub struct CardTarget {
    /// The entry, for the use record and the approval; its `part` is the number.
    pub fill: FillTarget,
    pub brand: &'static str,
    pub last_four: String,
    /// The expiry month and year, when the card has one.
    pub expiry: Option<(u32, i32)>,
    /// Past its month: asks every time, and no "Always" covers it.
    pub expired: bool,
    /// The person's default card.
    pub default: bool,
    /// The card's sealed bytes as the approval described them: a card changed while the person
    /// decides is not the card they said yes to.
    pub sealed: Vec<u8>,
}

impl CardTarget {
    /// This card as one line of the person's picker.
    pub fn choice(&self) -> super::uses::UseChoice {
        super::uses::UseChoice { id: self.fill.id.clone(), label: self.fill.label.clone(), detail: self.describe(), expired: self.expired, default: self.default }
    }

    /// What the person reads the card as: "Visa •••• 4242, expires 03/27".
    pub fn describe(&self) -> String {
        let brand = card_brand_name(self.brand);
        let tail = if self.last_four.is_empty() { String::new() } else { format!(" •••• {}", self.last_four) };
        let when = match self.expiry {
            Some((m, y)) if self.expired => format!(", expired {}/{y:04}", card::month_text(m)),
            Some((m, y)) => format!(", expires {}/{:02}", card::month_text(m), y % 100),
            None => String::new(),
        };
        format!("{}{tail}{when}", if brand.is_empty() { "card" } else { brand })
    }
}

fn card_brand_name(brand: &str) -> &'static str {
    match brand {
        "visa" => "Visa",
        "mastercard" => "Mastercard",
        "amex" => "American Express",
        "discover" => "Discover",
        "diners" => "Diners Club",
        "jcb" => "JCB",
        "unionpay" => "UnionPay",
        _ => "",
    }
}

/// What `vault` names for a whole-card fill.
pub enum Asked {
    /// Not a whole card (a part, another kind of entry): the old fill takes it.
    NotACard,
    /// A card: the one the agent named, or the person's default.
    One(Box<CardTarget>),
    /// `card` with no default: the person picks.
    Pick,
}

/// Whether `vault` names a whole card: a card's id with no part, or `card` for the default.
pub fn card_asked(store: &Store, vault: &str, agent: Option<&str>) -> Result<Asked, String> {
    let (id, part) = super::slots::split(vault);
    if part.is_some() {
        return Ok(Asked::NotACard);
    }
    let row = match store.vault_get(id).map_err(|e| e.to_string())? {
        Some(row) if row.kind != card::KIND => return Ok(Asked::NotACard),
        Some(row) => row,
        None if id == card::KIND => match default_card(store, agent)? {
            Some(row) => row,
            None if usable_cards(store, agent)?.is_empty() => return Err("you have no card saved: ask the person to add one in the Vault, or to type it (Vault request_value)".to_string()),
            None => return Ok(Asked::Pick),
        },
        None => return Ok(Asked::NotACard),
    };
    if !vaults::entry_row_usable(store, &row.vault_id, agent)? {
        return Err(format!("no Vault entry {id:?}: Vault's list action shows the entries"));
    }
    open_row(row).map(|c| Asked::One(Box::new(c)))
}

/// A card named by its id, among the Vaults the agent may use: what the person picked.
pub fn card_by_id(store: &Store, id: &str, agent: Option<&str>) -> Result<CardTarget, String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.filter(|r| r.kind == card::KIND);
    let row = row.ok_or_else(|| format!("no card {id:?}"))?;
    if !vaults::entry_row_usable(store, &row.vault_id, agent)? {
        return Err(format!("no card {id:?}"));
    }
    open_row(row)
}

/// Every card among the Vaults the agent may use, for the person's picker: the default first, then the
/// most recently saved.
pub fn card_choices(store: &Store, agent: Option<&str>) -> Result<Vec<CardTarget>, String> {
    let mut rows = usable_cards(store, agent)?;
    rows.sort_by(|a, b| is_default(b).cmp(&is_default(a)).then(b.updated_at.cmp(&a.updated_at)));
    rows.into_iter().map(open_row).collect()
}

fn usable_cards(store: &Store, agent: Option<&str>) -> Result<Vec<super::VaultRow>, String> {
    let mut out = Vec::new();
    for row in store.vault_list().map_err(|e| e.to_string())? {
        if row.kind == card::KIND && vaults::entry_row_usable(store, &row.vault_id, agent)? {
            out.push(row);
        }
    }
    Ok(out)
}

fn is_default(row: &super::VaultRow) -> bool {
    let fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
    fields.get(card::DEFAULT).map(String::as_str) == Some("yes")
}

/// A card row opened for what the approval says of it: brand, last four, expiry. The number itself leaves here only as
/// those and as the sealed bytes the approval was shown for.
fn open_row(row: super::VaultRow) -> Result<CardTarget, String> {
    let kind = kinds::kind(card::KIND).ok_or("the card kind is missing")?;
    let key = seal::key(false)?;
    let parts = seal::open(&key, &row.id, &row.sealed)?;
    let number = parts.get("number").and_then(card::normalize_number).unwrap_or_default();
    let last_four: String = number.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let expiry = card::expiry_of(&parts);
    let expired = expiry.is_some_and(|(m, y)| card::expired(m, y, super::expiry::today()));
    let default = is_default(&row);
    let sealed = row.sealed.clone();
    Ok(CardTarget {
        brand: card::brand(&number),
        last_four,
        expiry,
        expired,
        default,
        sealed,
        fill: FillTarget {
            id: row.id,
            kind: kind.name,
            typed: false,
            kind_info: kind,
            label: row.label,
            part: "number",
            slot: None,
            what: "card".to_string(),
            created_at: row.created_at,
            version: None,
        },
    })
}

/// The person's default card among the Vaults the agent may use: the most recently saved if two
/// say they are (one Vault keeps only one, but two Vaults each can).
fn default_card(store: &Store, agent: Option<&str>) -> Result<Option<super::VaultRow>, String> {
    let mut found: Option<super::VaultRow> = None;
    for row in usable_cards(store, agent)? {
        if is_default(&row) && found.as_ref().is_none_or(|f| row.updated_at > f.updated_at) {
            found = Some(row);
        }
    }
    Ok(found)
}

/// The card's parts for the host to type, opened only once the person has said yes; never
/// returned to an agent. Refused if the entry was replaced, or its card changed, while they decided.
pub fn parts_for_fill(store: &Store, id: &str, created_at: i64, sealed: &[u8]) -> Result<seal::Parts, String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    if row.created_at != created_at {
        return Err(format!("{} was replaced while the person was deciding: nothing was typed", row.label));
    }
    if row.sealed != sealed {
        return Err(format!("{} was changed while the person was deciding, so it isn't the card they approved: nothing was typed", row.label));
    }
    let key = seal::key(false)?;
    seal::open(&key, &row.id, &row.sealed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(brand: &'static str, last_four: &str, expiry: Option<(u32, i32)>, expired: bool) -> CardTarget {
        let kind = kinds::kind(card::KIND).expect("card kind");
        CardTarget {
            fill: FillTarget { id: "card-a".into(), kind: kind.name, typed: false, kind_info: kind, label: "A".into(), part: "number", slot: None, what: "card".into(), created_at: 1, version: None },
            brand,
            last_four: last_four.into(),
            expiry,
            expired,
            default: false,
            sealed: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_card_changed_while_the_person_decides_is_not_filled() {
        use super::super::{create, update, EntryChange, NewEntry};
        seal::use_test_key();
        let store = crate::server::tests::test_state().identity_store.clone();
        let parts = |n: &str| seal::Parts([("number".to_string(), n.to_string()), ("exp_month".to_string(), "03".to_string()), ("exp_year".to_string(), "2099".to_string())].into());
        let id = create(&store, NewEntry { kind: "card".into(), label: "Probe".into(), fields: Default::default(), tags: vec![], parts: parts("4111111111111111") }).unwrap();
        let Asked::One(asked) = card_asked(&store, &id, None).unwrap() else { panic!("a card") };
        assert_eq!(asked.last_four, "1111");
        // Unchanged: the fill goes ahead with the card that was shown.
        assert!(parts_for_fill(&store, &id, asked.fill.created_at, &asked.sealed).is_ok());
        // The person changes the number while the prompt is open: what was approved is not what is there.
        update(&store, EntryChange { id: id.clone(), label: "Probe".into(), fields: Default::default(), tags: vec![], parts: parts("5555555555554444"), clear: vec![] }).unwrap();
        let why = parts_for_fill(&store, &id, asked.fill.created_at, &asked.sealed).err().expect("refused");
        assert!(why.contains("changed while the person was deciding"), "{why}");
        assert!(!why.contains("5555") && !why.contains("4111"), "{why}");
    }

    #[test]
    fn a_card_is_described_by_its_brand_last_four_and_expiry() {
        assert_eq!(target("visa", "4242", Some((3, 2027)), false).describe(), "Visa •••• 4242, expires 03/27");
        assert_eq!(target("amex", "0005", Some((12, 2030)), false).describe(), "American Express •••• 0005, expires 12/30");
        assert_eq!(target("visa", "4242", Some((3, 2026)), true).describe(), "Visa •••• 4242, expired 03/2026");
        assert_eq!(target("", "", None, false).describe(), "card");
    }

    #[tokio::test]
    async fn card_names_the_default_or_asks_the_person_to_pick_and_lists_the_default_first() {
        use super::super::{create, NewEntry};
        seal::use_test_key();
        let store = crate::server::tests::test_state().identity_store.clone();
        assert!(matches!(card_asked(&store, "card", None), Err(e) if e.contains("no card saved")));
        let make = |label: &str, number: &str, default: bool| {
            let fields = if default { [("default".to_string(), "yes".to_string())].into() } else { Default::default() };
            let parts = seal::Parts([("number", number), ("exp_month", "03"), ("exp_year", "2099")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
            create(&store, NewEntry { kind: "card".into(), label: label.into(), fields, tags: vec![], parts }).unwrap()
        };
        let plain = make("Plain", "4111111111111111", false);
        // Cards but no default: the person picks.
        assert!(matches!(card_asked(&store, "card", None), Ok(Asked::Pick)));
        let default = make("Default", "5555555555554444", true);
        let Ok(Asked::One(named)) = card_asked(&store, "card", None) else { panic!("the default") };
        assert_eq!(named.fill.id, default);
        let all = card_choices(&store, None).unwrap();
        assert_eq!(all.iter().map(|c| c.fill.id.as_str()).collect::<Vec<_>>(), [default.as_str(), plain.as_str()], "the default first");
        assert!(all[0].choice().default && !all[1].choice().default);
        assert_eq!(all[1].choice().detail, "Visa •••• 1111, expires 03/99");
        assert!(card_by_id(&store, &plain, None).is_ok());
        assert!(card_by_id(&store, "nope", None).is_err());
    }
}
