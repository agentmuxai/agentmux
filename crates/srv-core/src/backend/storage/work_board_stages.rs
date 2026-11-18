// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The stages of a Kanban board: the named, coloured columns the person sets
//! (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §4, §5).
//!
//! A stage shows some of the board's eight built-in *sources* (`backlog` … `released`, the columns
//! the board computes from a task's state, kind, PR and holder). A task still lands where its state
//! puts it; the stages only say which stage shows which source. Nothing about a task changes.
//!
//! This module is the pure part: the types, the default set and the validator. The board's row keeps
//! the set as JSON text (`db_boards.stages`); empty means the default set.

use serde::{Deserialize, Serialize};

/// The only version of the stored set this build reads and writes.
pub const STAGES_VERSION: i64 = 1;
/// The fewest stages: Needs you, and one that shows everything else (R1).
pub const MIN_STAGES: usize = 2;
/// The most stages. Without a split by kind there is one stage per source (R2), so eight; the split
/// (spec phase B2) raises it to twelve.
pub const MAX_STAGES: usize = 8;
/// The longest stage name, in characters.
pub const MAX_STAGE_NAME: usize = 24;
/// The longest stage id.
const MAX_STAGE_ID: usize = 24;

/// The sources: the eight columns the board computes. Fixed, and the ids a pane's collapsed set stores.
pub const SOURCES: [&str; 8] = ["backlog", "research", "specd", "progress", "needs", "review", "merged", "released"];
/// The source that asks the operator for something; its stage stays whole (R4).
pub const NEEDS: &str = "needs";
/// The words that belong to the stage taking `needs`.
const NEEDS_NAME: &str = "needs you";
/// The palette: names of the theme colours a stage may use (spec §9.1).
pub const COLOURS: [&str; 8] = ["grey", "cyan", "violet", "magenta", "amber", "yellow", "green", "blue"];

/// One stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub name: String,
    pub colour: String,
    /// The sources it shows, at least one.
    pub takes: Vec<String>,
    /// A split by task kind (spec phase B2). Refused until then.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
}

/// A board's stages, in the order they are drawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageSet {
    pub version: i64,
    pub stages: Vec<Stage>,
}

/// Today's board: one stage per source, with today's names and colours.
pub fn default_set() -> StageSet {
    let stage = |id: &str, name: &str, colour: &str| Stage {
        id: id.into(),
        name: name.into(),
        colour: colour.into(),
        takes: vec![id.into()],
        kinds: Vec::new(),
    };
    StageSet {
        version: STAGES_VERSION,
        stages: vec![
            stage("backlog", "Backlog", "grey"),
            stage("research", "In Research", "cyan"),
            stage("specd", "Spec'd", "violet"),
            stage("progress", "In progress", "magenta"),
            stage("needs", "Needs you", "amber"),
            stage("review", "Review", "yellow"),
            stage("merged", "Merged", "green"),
            stage("released", "Released", "blue"),
        ],
    }
}

/// Trim every name. Names are the person's text: nothing else is changed.
pub fn clean(mut set: StageSet) -> StageSet {
    for s in &mut set.stages {
        s.name = s.name.trim().to_string();
        s.id = s.id.trim().to_string();
        s.colour = s.colour.trim().to_ascii_lowercase();
        s.kinds = s.kinds.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
        let mut seen = Vec::new();
        s.takes.retain(|t| !seen.contains(t) && { seen.push(t.clone()); true });
    }
    set
}

/// Every rule of the spec (§5), each broken one as a sentence for the person or agent. Empty means the
/// set is good. srv never repairs a set: it refuses with these.
pub fn validate(set: &StageSet) -> Vec<String> {
    let mut why = Vec::new();
    if set.version != STAGES_VERSION {
        why.push(format!("stages version {} is not one this AgentMux reads (it reads {STAGES_VERSION})", set.version));
        return why;
    }
    let n = set.stages.len();
    if n < MIN_STAGES {
        why.push(format!("a board needs at least {MIN_STAGES} stages"));
    }
    if n > MAX_STAGES {
        why.push(format!("a board has at most {MAX_STAGES} stages"));
    }
    for (i, s) in set.stages.iter().enumerate() {
        let at = |what: String| format!("stage {}: {what}", i + 1);
        check_id(set, i, &at, &mut why);
        check_name(set, i, &at, &mut why);
        if !COLOURS.contains(&s.colour.as_str()) {
            why.push(at(format!("colour \"{}\" is not one of {}", s.colour, COLOURS.join(", "))));
        }
        if !s.kinds.is_empty() {
            why.push(at("splitting a stage by task kind is not available yet".into()));
        }
        if s.takes.is_empty() {
            why.push(at("it must show at least one source".into()));
        }
        for src in &s.takes {
            if !SOURCES.contains(&src.as_str()) {
                why.push(at(format!("\"{src}\" is not a source (the sources are {})", SOURCES.join(", "))));
            }
        }
    }
    check_sources(set, &mut why);
    why
}

fn check_id(set: &StageSet, i: usize, at: &dyn Fn(String) -> String, why: &mut Vec<String>) {
    let id = &set.stages[i].id;
    let ok = !id.is_empty() && id.chars().count() <= MAX_STAGE_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !ok {
        why.push(at(format!("its id \"{id}\" is letters, digits and hyphens, up to {MAX_STAGE_ID}")));
    } else if set.stages[..i].iter().any(|o| o.id == *id) {
        why.push(at(format!("its id \"{id}\" is used by another stage")));
    }
}

fn check_name(set: &StageSet, i: usize, at: &dyn Fn(String) -> String, why: &mut Vec<String>) {
    let s = &set.stages[i];
    let name = s.name.trim();
    if name.is_empty() {
        why.push(at("it needs a name".into()));
        return;
    }
    if name.chars().count() > MAX_STAGE_NAME {
        why.push(at(format!("its name is at most {MAX_STAGE_NAME} characters")));
    }
    if name.chars().any(char::is_control) {
        why.push(at("its name has a control character".into()));
    }
    let key = name.to_lowercase();
    if set.stages[..i].iter().any(|o| o.name.trim().to_lowercase() == key) {
        why.push(at(format!("the name \"{name}\" is used by another stage")));
    }
    if key == NEEDS_NAME && !s.takes.iter().any(|t| t == NEEDS) {
        why.push(at("\"Needs you\" is the name of the stage that shows what needs the operator".into()));
    }
}

/// R2 and R4 without a split by kind: every source is shown by exactly one stage, and `needs` by one
/// that shows nothing else.
fn check_sources(set: &StageSet, why: &mut Vec<String>) {
    for src in SOURCES {
        let shown = set.stages.iter().filter(|s| s.takes.iter().any(|t| t == src)).count();
        match shown {
            0 => why.push(format!("no stage shows {src}; every source needs one")),
            1 => {}
            _ => why.push(format!("{src} is shown by {shown} stages; each source is shown by exactly one")),
        }
    }
    for s in &set.stages {
        let n = s.takes.iter().filter(|t| t.as_str() == NEEDS).count();
        if n > 0 && s.takes.len() != n {
            why.push(format!("the stage \"{}\" shows needs, which stays a stage of its own", s.name.trim()));
        }
    }
}

/// The stored text of a board's stages as a validated set: empty is the default set. `Err` carries
/// why a stored set can't be used (a version this build doesn't know, or a rule it breaks).
pub fn parse_stored(text: &str) -> Result<StageSet, String> {
    if text.trim().is_empty() {
        return Ok(default_set());
    }
    let set: StageSet = serde_json::from_str(text).map_err(|e| format!("the stored stages can't be read: {e}"))?;
    let why = validate(&set);
    if why.is_empty() { Ok(set) } else { Err(why.join("; ")) }
}

#[cfg(test)]
#[path = "work_board_stages_tests.rs"]
mod tests;
