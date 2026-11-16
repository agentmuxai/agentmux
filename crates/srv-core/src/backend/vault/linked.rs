// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A linked entry: an entry that says where a key lives and who reads it, and holds no value
//! (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2).
//!
//! A key that a machine role reads must stay where that role can reach it, so it never gets into the Vault. A linked
//! entry lets the Vault say that the key exists, where it is, who reads it, when it expires and whether it changed. It
//! is an ordinary entry of an ordinary kind, marked by the plain field `link` set to `only`, with the place in the
//! machine copy's fields ([`super::source_link`]). Its check reads the place's description and never the key, so no
//! value reaches srv and nothing needs the person's yes.

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;
use chrono::NaiveDate;
use serde::Serialize;

use super::aws_import::{self, Described, Imported, Plan, Source, Target, Via, NO_SUCH_SECRET};
use super::source_link::{self, CheckPlan, Checked, Link, Outcome, VaultSourceCheck, AWS_SECRET, GITHUB_SECRET, OTHER};
use super::{expiry, history, insert_unrecorded, record_line, record_row, signins, Line, NewEntry, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};
use crate::identity::aws_broker::mint::Runner;

/// The field that marks a linked entry, and the one thing it may say.
pub const LINK: &str = "link";
pub const ONLY: &str = "only";
/// Who or what reads the key, one line each: `invoice-sync (lambda): at run time`. Free text; the Vault enforces none of it.
pub const READERS: &str = "readers";
/// When the key was last changed, as recorded: a date. A linked entry's own, since the Vault holds no value to date.
pub const ROTATED: &str = "rotated";
/// Other places the same key is kept, one line each: `kind: location#key`.
pub const COPIES: &str = "copies";
pub const FIELDS: [&str; 4] = [LINK, READERS, ROTATED, COPIES];
/// What is said of the key, as a KeePass export names it, so a database written from the Vault brings it back.
pub const TITLES: [(&str, &str); 3] = [(READERS, "Read by"), (ROTATED, "Last changed"), (COPIES, "Also kept at")];
/// The KeePass field that marks a linked entry. It holds the entry's kind: a link has no secret field to say it.
pub const KIND_TITLE: &str = "Linked entry kind";
pub const MAX_READERS: usize = 8;
pub const MAX_COPIES: usize = 4;
const MAX_LINE: usize = 200;

/// The Activity action of an entry made as a link.
pub const LINKED: &str = "linked";
/// The Activity action of a check of one of a linked entry's other places. Its own action, so the newest `checked`
/// line is always the entry's own place, and an agent's check counts once toward the hour's limit.
pub const CHECKED_COPY: &str = "checked a copy";
/// A linked entry nobody has checked for this many days is counted as unchecked.
pub const UNCHECKED_DAYS: i64 = 30;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Why a change that would make an entry a link, or a link an entry with a value, is refused.
pub const SET_WHEN_MADE: &str = "whether an entry is a link (it holds no value) is set when it is made: make a new entry instead";
const NOT_FROM_HERE: &str = "this kind of place can't be described from here: only an AWS secret is, today";
const NOT_UNDER_AWS: &str = "an AWS copy is described under the entry's own AWS place, and this entry's place isn't in AWS";

/// Whether `name` is one of a link's fields, which every kind of entry may have.
pub fn is_field(name: &str) -> bool {
    FIELDS.contains(&name)
}

/// Whether the entry with these fields is a link: it holds no value.
pub fn is_linked(fields: &BTreeMap<String, String>) -> bool {
    fields.get(LINK).is_some_and(|v| v == ONLY)
}

fn fields_of(row: &VaultRow) -> BTreeMap<String, String> {
    serde_json::from_str(&row.fields).unwrap_or_default()
}

pub fn is_linked_row(row: &VaultRow) -> bool {
    is_linked(&fields_of(row))
}

/// The lines of a field that holds one thing a line (`readers`, `copies`), trimmed, without the empty ones.
pub fn lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// A place as a line of `copies` writes it, and as a refusal names it: `aws-secret: staging/app#db.password`.
pub fn place_text(link: &Link) -> String {
    match link.key.as_str() {
        "" => format!("{}: {}", link.kind, link.location),
        key => format!("{}: {}#{key}", link.kind, link.location),
    }
}

/// The place a line of `copies` names, held to what that kind of place may be.
fn copy_of(line: &str) -> Result<Link, String> {
    let wrong = |why: &str| format!("a line of {COPIES} is `kind: location#key`, with the kind {AWS_SECRET}, {GITHUB_SECRET} or {OTHER}; {:?} isn't one ({why})", history::one_line(line, 60));
    let (kind, rest) = line.split_once(':').ok_or_else(|| wrong("it names no kind"))?;
    let (location, key) = rest.rsplit_once('#').unwrap_or((rest, ""));
    let named = [(source_link::KIND, kind), (source_link::LOCATION, location), (source_link::KEY, key)];
    let fields: BTreeMap<String, String> = named.into_iter().map(|(k, v)| (k.to_string(), v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
    source_link::validate(&fields).map_err(|why| wrong(&why))?;
    Link::of(&fields).ok_or_else(|| wrong("it names no place"))
}

/// The other places an entry says its key is kept, in order. A line that names no place is left out: `validate`
/// refuses one before it is saved.
pub fn copies(fields: &BTreeMap<String, String>) -> Vec<Link> {
    lines(fields.get(COPIES).map_or("", String::as_str)).into_iter().filter_map(|l| copy_of(l).ok()).collect()
}

/// Whether the link's fields in `fields` are ones the Vault keeps, for an entry that holds a value (`has_value`) or
/// none. A linked entry names its place and holds nothing; who reads a key and where else it is kept may be said of
/// any entry.
pub fn validate(fields: &BTreeMap<String, String>, has_value: bool) -> Result<(), String> {
    let linked = match fields.get(LINK).map(String::as_str) {
        None => false,
        Some(ONLY) => true,
        Some(_) => return Err(format!("{LINK} is \"{ONLY}\", for an entry that records where a key lives and holds no value, or is left out")),
    };
    if linked && Link::of(fields).is_none() {
        return Err(format!("a linked entry says where the key lives: set {} and {}", source_link::KIND, source_link::LOCATION));
    }
    if linked && has_value {
        return Err("a linked entry holds no value: leave the value out, or leave link out to keep the value in the Vault".into());
    }
    // A saved sign-in is offered by its site and filled with its password, which a link does not have.
    if linked && fields.contains_key(signins::SITE) {
        return Err(format!("a linked entry holds no password, so it is not a saved sign-in: leave {} out", signins::SITE));
    }
    if let Some(date) = fields.get(ROTATED) {
        if !linked {
            return Err(format!("{ROTATED} is for a linked entry: the Vault's own history says when a value it holds was changed"));
        }
        expiry::parse(date).map_err(|_| format!("{ROTATED} is the date the key was last changed, as YYYY-MM-DD"))?;
    }
    let readers = lines(fields.get(READERS).map_or("", String::as_str));
    if readers.len() > MAX_READERS || readers.iter().any(|l| l.chars().count() > MAX_LINE) {
        return Err(format!("{READERS} is who or what reads the key, one line each: at most {MAX_READERS} lines of {MAX_LINE} characters"));
    }
    let places = lines(fields.get(COPIES).map_or("", String::as_str));
    if places.len() > MAX_COPIES {
        return Err(format!("{COPIES} is the other places the key is kept, one line each: at most {MAX_COPIES}"));
    }
    places.into_iter().try_for_each(|line| copy_of(line).map(|_| ()))
}

/// Why an agent can't be given this entry's value, when the entry is a link: it has none. Nothing is asked of the
/// person, because there is nothing to reveal.
pub fn no_value(row: &VaultRow) -> Option<String> {
    let fields = fields_of(row);
    if !is_linked(&fields) {
        return None;
    }
    let place = Link::of(&fields).map(|l| place_text(&l)).unwrap_or_default();
    Some(format!("This entry holds no value: the key is kept at {place}. Ask the person, or use the access you already have there."))
}

/// Why `row` can't be given a value (a rotation, a capture, an import into it), when it is a link; `instead` says
/// what to do.
pub fn takes_no_value(row: &VaultRow, instead: &str) -> Option<String> {
    is_linked_row(row).then(|| format!("{} is a linked entry, which holds no value: {instead}", row.label))
}

/// What an entry records about its key beside the place, as the person reads it before an agent changes it: who
/// reads it, when it last changed, where else it is kept, one of them a line, so the card can show each on its own
/// line and mark the ones a change touches. Empty when it records none of them.
pub fn about(fields: &BTreeMap<String, String>) -> String {
    let joined = |name: &str| lines(fields.get(name).map_or("", String::as_str)).join("; ");
    let said = [("Read by", joined(READERS)), ("Last changed", joined(ROTATED)), ("Also kept at", joined(COPIES))];
    said.into_iter().filter(|(_, text)| !text.is_empty()).map(|(what, text)| format!("{what}: {text}.")).collect::<Vec<_>>().join("\n")
}

/// The fields of a linked entry for the secret at `source`: the mark, the place, and what the asker says of it
/// (`said`: readers, rotated, copies, an expiry date, the kind's own plain fields). The place is `source`, whatever
/// `said` names.
pub fn fields_for(source: &Source, said: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut fields: BTreeMap<String, String> = said.iter().filter(|(k, _)| !source_link::is_field(k)).map(|(k, v)| (k.clone(), v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
    let place = [
        (LINK, Some(ONLY)),
        (source_link::KIND, Some(AWS_SECRET)),
        (source_link::LOCATION, Some(source.aws_secret_id.as_str())),
        (source_link::KEY, source.key.as_deref()),
        (source_link::REGION, source.region.as_deref()),
        (source_link::PROFILE, source.profile.as_deref()),
    ];
    fields.extend(place.into_iter().filter_map(|(k, v)| Some((k.to_string(), v?.to_string()))));
    fields
}

/// The record of a link a batch made, or did not make (`done`, `failed`): who, which place, why. Nothing was read.
pub fn note(store: &Store, plan: &Plan, id: &str, label: &str, agent: &str, outcome: &str) {
    let place = match &plan.target {
        Target::Link { fields, .. } => Link::of(fields).map(|l| l.text()).unwrap_or_default(),
        _ => plan.from_text(),
    };
    record_row(
        store,
        VaultActivityRow {
            entry_id: id.into(),
            entry_label: label.into(),
            actor: agent.into(),
            action: LINKED.into(),
            target: place,
            outcome: outcome.into(),
            reason: plan.reason.clone(),
            refs: history::refs_text(&plan.refs),
            ..Default::default()
        },
    );
}

/// Makes the linked entry a batch's link item asked for. Nothing is read: the entry is the place and what is said of
/// it. Its line carries the agent's reason and the batch's references.
pub fn make(store: &Store, plan: &Plan, agent: &str) -> Result<Imported, String> {
    let Target::Link { kind, label, fields, vault_id } = &plan.target else {
        return Err(format!("{} is not a link", plan.label()));
    };
    let entry = NewEntry { kind: kind.to_string(), label: label.clone(), fields: fields.clone(), tags: Vec::new(), parts: Parts::default() };
    let (id, label) = insert_unrecorded(store, entry, vault_id)?;
    note(store, plan, &id, &label, agent, "done");
    Ok(Imported { id, label, vault: plan.vault_name.clone() })
}

/// What a check found at one of a linked entry's other places.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CopyChecked {
    /// The place, as the entry's `copies` line writes it.
    pub place: String,
    /// `present`, `changed since recorded`, `missing` or `could not read`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// What is known of one of the other places an entry names, for its "Also kept at" line. No value.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultCopyCheck {
    /// The place, as the entry's `copies` line writes it.
    pub place: String,
    /// The newest check of that place as the entry is now; absent when it was never checked, or can't be.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub checked: Option<VaultSourceCheck>,
    /// Why no check can describe this place from here; empty when one can.
    pub cannot: String,
}

/// Why a check can't describe `copy` from here, for an entry whose own place is of the kind `own`; `None` when it
/// can: a copy in AWS is described under the entry's own AWS place, and nothing else is, today.
fn cannot_describe(copy: &Link, own: &str) -> Option<&'static str> {
    match (copy.kind.as_str(), own) {
        (AWS_SECRET, AWS_SECRET) => None,
        (AWS_SECRET, _) => Some(NOT_UNDER_AWS),
        _ => Some(NOT_FROM_HERE),
    }
}

/// Each other place the entry names, with the newest check of it as the entry is now. A check leaves a line for a
/// copy it can describe; for one it can't, that is said here in place of a check, so the pane shows it and not
/// only the answer the asking agent got.
pub fn copy_checks(store: &Store, row: &VaultRow) -> Vec<VaultCopyCheck> {
    let fields = fields_of(row);
    // Each line with the place it names: the pane finds a place's check by the line as the entry writes it.
    let named: Vec<(&str, Link)> = lines(fields.get(COPIES).map_or("", String::as_str)).into_iter().filter_map(|line| Some((line, copy_of(line).ok()?))).collect();
    if named.is_empty() {
        return Vec::new();
    }
    let own = Link::of(&fields).map(|link| link.kind).unwrap_or_default();
    // Newest first. A check writes one line a copy, so this holds the last few checks of every copy.
    let checks = store.vault_activity_for_entry(&row.id, &[CHECKED_COPY], row.updated_at, (MAX_COPIES * 4) as i64).unwrap_or_default();
    named
        .iter()
        .map(|(written, copy)| match cannot_describe(copy, &own) {
            Some(why) => VaultCopyCheck { place: written.to_string(), checked: None, cannot: why.into() },
            None => {
                let checked = checks.iter().find(|line| line.target == copy.text()).map(|line| VaultSourceCheck { at: line.at, outcome: line.outcome.clone() });
                VaultCopyCheck { place: written.to_string(), checked, cannot: String::new() }
            }
        })
        .collect()
}

/// What each place's description came to: the entry's own place, then its other places in order. No value is in a
/// description.
pub struct Found {
    place: Result<Described, String>,
    copies: Vec<(Link, Result<Described, String>)>,
}

async fn describe_place<R: Runner>(runner: &R, link: &Link, via: Via<'_>) -> Result<Described, String> {
    let Some(source) = link.aws() else {
        return Err(NOT_FROM_HERE.into());
    };
    aws_import::describe_secret(runner, &source, via).await
}

/// Reads the description of the entry's place and of each other place it names, with the asker's access. A copy in
/// AWS is described as the entry's own place is, under the same region and login: when the entry's place is not in
/// AWS there is no login to describe one under, and nothing is run for it.
pub async fn describe<R: Runner>(runner: &R, plan: &CheckPlan, via: Via<'_>) -> Found {
    // The places are described together: each run of the CLI has its own time limit, and a check of an entry with
    // four copies must still answer inside the caller's wait.
    let copies = plan.copies.iter().map(|copy| async move {
        let described = match cannot_describe(copy, &plan.link.kind) {
            None => describe_place(runner, &Link { region: plan.link.region.clone(), profile: plan.link.profile.clone(), ..copy.clone() }, via).await,
            Some(why) => Err(why.into()),
        };
        (copy.clone(), described)
    });
    let (place, copies) = futures_util::future::join(describe_place(runner, &plan.link, via), futures_util::future::join_all(copies)).await;
    Found { place, copies }
}

/// What a place's description says against the date the entry records: gone, changed after that date, or there.
fn outcome_of(described: &Result<Described, String>, recorded: Option<NaiveDate>) -> Outcome {
    match (described, recorded) {
        (Err(why), _) if why == NO_SUCH_SECRET => Outcome::Missing(why.clone()),
        (Err(why), _) => Outcome::CouldNotRead(why.clone()),
        (Ok(d), _) if d.deleted => Outcome::Missing("the secret is scheduled for deletion".into()),
        (Ok(Described { changed: Some(changed), .. }), Some(recorded)) if *changed > recorded => Outcome::Changed(format!("the place last changed on {changed}, after the {recorded} recorded")),
        (Ok(_), _) => Outcome::Present,
    }
}

/// Records what the descriptions came to and answers it: a line for the entry's own place, and one for each other
/// place that was asked. Never fails: a place that can't be described is an outcome.
pub fn finish(store: &Store, plan: &CheckPlan, found: Found) -> Checked {
    let row = match source_link::as_planned(store, plan) {
        Ok(row) => row,
        Err(why) => return source_link::conclude(store, plan, &Outcome::CouldNotRead(why)),
    };
    let recorded = fields_of(&row).get(ROTATED).and_then(|date| expiry::parse(date).ok());
    let mut copies = Vec::with_capacity(found.copies.len());
    for (copy, described) in &found.copies {
        let outcome = outcome_of(described, recorded);
        if cannot_describe(copy, &plan.link.kind).is_none() {
            record_line(store, Line { entry_id: &plan.entry_id, label: &plan.label, actor: &plan.actor, action: CHECKED_COPY, target: &copy.text(), outcome: &outcome.line(), reason: &plan.reason });
        }
        copies.push(CopyChecked { place: place_text(copy), outcome: outcome.word(), why: outcome.why().to_string() });
    }
    let mut checked = source_link::conclude(store, plan, &outcome_of(&found.place, recorded));
    if let Ok(described) = &found.place {
        checked.last_changed = described.changed.map(|day| day.to_string());
        checked.rotation = described.rotation;
    }
    checked.copies = copies;
    checked
}

/// How a linked entry stands, by its newest check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Standing {
    /// Checked within [`UNCHECKED_DAYS`], and the place was there and unchanged, or could not be described.
    Checked,
    /// The newest check found the place changed since the date recorded, or missing.
    NeedsALook,
    /// Never checked as it is now, or not for [`UNCHECKED_DAYS`].
    Unchecked,
}

/// How a link stands by its newest check as it is now, and whether it has gone unchecked: that check is older than
/// [`UNCHECKED_DAYS`], or there is none. A place found changed or missing that long ago is both.
fn stands(checked: Option<&VaultSourceCheck>, now: i64) -> (Standing, bool) {
    let unchecked = checked.is_none_or(|c| now - c.at > UNCHECKED_DAYS * DAY_MS);
    let how = match checked {
        Some(c) if c.outcome.starts_with(source_link::CHANGED) || c.outcome.starts_with(source_link::MISSING) => Standing::NeedsALook,
        Some(_) if !unchecked => Standing::Checked,
        _ => Standing::Unchecked,
    };
    (how, unchecked)
}

/// How `row` stands and whether it has gone unchecked ([`stands`]), when it is a linked entry.
pub fn standing(store: &Store, row: &VaultRow) -> Option<(Standing, bool)> {
    let fields = fields_of(row);
    if !is_linked(&fields) {
        return None;
    }
    let checked = Link::of(&fields).and_then(|link| source_link::last_check(store, row, &link));
    Some(stands(checked.as_ref(), now_ms()))
}

#[cfg(test)]
#[path = "linked_tests.rs"]
mod tests;
