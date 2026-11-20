// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn stage(id: &str, name: &str, colour: &str, takes: &[&str]) -> Stage {
    Stage { id: id.into(), name: name.into(), colour: colour.into(), takes: takes.iter().map(|t| t.to_string()).collect(), kinds: vec![] }
}

/// Needs you, and one stage that shows everything else: the fewest stages.
fn minimal() -> StageSet {
    StageSet {
        version: STAGES_VERSION,
        stages: vec![
            stage("needs", "Needs you", "attention", &["needs"]),
            stage("st-all", "Everything", "muted", &["backlog", "research", "specd", "progress", "review", "merged", "released"]),
        ],
    }
}

fn reasons(set: &StageSet) -> String {
    validate(&clean(set.clone())).join(" | ")
}

#[test]
fn the_default_set_is_valid_and_is_todays_eight_columns() {
    let set = default_set();
    assert!(validate(&set).is_empty(), "{:?}", validate(&set));
    let ids: Vec<&str> = set.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, SOURCES);
    assert!(set.stages.iter().all(|s| s.takes == vec![s.id.clone()]));
}

#[test]
fn merging_sources_into_one_stage_is_valid() {
    let mut set = default_set();
    set.stages.retain(|s| s.id != "released");
    set.stages.iter_mut().find(|s| s.id == "merged").unwrap().takes.push("released".into());
    set.stages.iter_mut().find(|s| s.id == "merged").unwrap().name = "Done".into();
    assert!(validate(&set).is_empty(), "{:?}", validate(&set));
    assert!(validate(&minimal()).is_empty());
}

#[test]
fn r1_two_to_eight_stages() {
    let mut one = minimal();
    one.stages.truncate(1);
    assert!(reasons(&one).contains("at least 2 stages"), "{}", reasons(&one));
    let mut nine = default_set();
    nine.stages.push(stage("st-extra", "Extra", "muted", &["backlog"]));
    assert!(reasons(&nine).contains("at most 8 stages"), "{}", reasons(&nine));
}

#[test]
fn r2_every_source_is_shown_by_exactly_one_stage() {
    let mut set = minimal();
    set.stages[1].takes.retain(|t| t != "review");
    assert!(reasons(&set).contains("no stage shows review"), "{}", reasons(&set));
    let mut twice = default_set();
    twice.stages.iter_mut().find(|s| s.id == "backlog").unwrap().takes.push("review".into());
    assert!(reasons(&twice).contains("review is shown by 2 stages"), "{}", reasons(&twice));
}

#[test]
fn r4_needs_stays_a_stage_of_its_own() {
    let mut set = minimal();
    set.stages[0].takes.push("review".into());
    set.stages[1].takes.retain(|t| t != "review");
    assert!(reasons(&set).contains("shows needs, which stays a stage of its own"), "{}", reasons(&set));
    let mut merged = minimal();
    merged.stages.remove(0);
    merged.stages[0].takes.push("needs".into());
    merged.stages.push(stage("st-two", "Two", "blue", &["released"]));
    merged.stages[0].takes.retain(|t| t != "released");
    assert!(reasons(&merged).contains("stays a stage of its own"), "{}", reasons(&merged));
}

#[test]
fn r5_a_stage_shows_at_least_one_source() {
    let mut set = minimal();
    set.stages.push(stage("st-empty", "Empty", "blue", &[]));
    assert!(reasons(&set).contains("at least one source"), "{}", reasons(&set));
}

#[test]
fn r6_ids_are_unique_and_plain() {
    let mut set = default_set();
    set.stages[1].id = "backlog".into();
    assert!(reasons(&set).contains("its id \"backlog\" is used by another stage"), "{}", reasons(&set));
    let mut odd = minimal();
    odd.stages[1].id = "has space".into();
    assert!(reasons(&odd).contains("letters, digits and hyphens"), "{}", reasons(&odd));
}

#[test]
fn names_are_trimmed_unique_and_bounded() {
    let mut set = minimal();
    set.stages[1].name = "   ".into();
    assert!(reasons(&set).contains("needs a name"), "{}", reasons(&set));
    set.stages[1].name = "x".repeat(MAX_STAGE_NAME + 1);
    assert!(reasons(&set).contains("at most 24 characters"), "{}", reasons(&set));
    set.stages[1].name = "x".repeat(MAX_STAGE_NAME);
    assert!(reasons(&set).is_empty(), "{}", reasons(&set));
    set.stages[1].name = "needs you".into();
    assert!(reasons(&set).contains("used by another stage"), "{}", reasons(&set));
    set.stages[1].name = "bad\u{7}name".into();
    assert!(reasons(&set).contains("control character"), "{}", reasons(&set));
}

#[test]
fn a_name_that_differs_only_in_case_or_padding_collides() {
    let mut set = default_set();
    set.stages[0].name = " done".into();
    set.stages[1].name = "DONE ".into();
    assert!(reasons(&set).contains("used by another stage"), "{}", reasons(&set));
}

#[test]
fn only_the_stage_that_shows_needs_may_be_called_needs_you() {
    let mut set = minimal();
    set.stages[1].name = "Needs You".into();
    assert!(reasons(&set).contains("name of the stage that shows what needs the operator"), "{}", reasons(&set));
    let mut renamed = minimal();
    renamed.stages[0].name = "Ask me".into();
    assert!(reasons(&renamed).is_empty(), "{}", reasons(&renamed));
}

#[test]
fn any_script_emoji_and_right_to_left_names_are_fine() {
    let mut set = minimal();
    set.stages[0].name = "דחוף".into();
    set.stages[1].name = "完了 ✅".into();
    assert!(reasons(&set).is_empty(), "{}", reasons(&set));
}

#[test]
fn colours_come_from_the_palette() {
    let mut set = minimal();
    set.stages[1].colour = "#ff0000".into();
    assert!(reasons(&set).contains("is not one of"), "{}", reasons(&set));
    set.stages[1].colour = "Amber".into();
    assert!(reasons(&set).is_empty(), "a palette name is case-insensitive and shared colours are allowed: {}", reasons(&set));
}

#[test]
fn a_split_by_kind_is_refused_until_it_exists() {
    let mut set = minimal();
    set.stages[1].kinds = vec!["spec".into()];
    assert!(reasons(&set).contains("by task kind is not available yet"), "{}", reasons(&set));
}

#[test]
fn an_unknown_source_or_version_is_refused() {
    let mut set = minimal();
    set.stages[1].takes.push("later".into());
    assert!(reasons(&set).contains("\"later\" is not a source"), "{}", reasons(&set));
    let mut v2 = minimal();
    v2.version = 2;
    assert!(reasons(&v2).contains("version 2"), "{}", reasons(&v2));
}

#[test]
fn a_stored_set_is_the_default_when_empty_and_an_error_when_unusable() {
    assert_eq!(parse_stored("").unwrap(), default_set());
    assert_eq!(parse_stored("  ").unwrap(), default_set());
    let text = serde_json::to_string(&minimal()).unwrap();
    assert_eq!(parse_stored(&text).unwrap(), minimal());
    assert!(parse_stored("{not json").unwrap_err().contains("can't be read"));
    let mut broken = minimal();
    broken.stages[1].takes.clear();
    assert!(parse_stored(&serde_json::to_string(&broken).unwrap()).is_err());
}

#[test]
fn clean_trims_names_and_drops_repeated_sources() {
    let mut set = minimal();
    set.stages[1].name = "  Everything  ".into();
    set.stages[1].takes.push("backlog".into());
    let cleaned = clean(set);
    assert_eq!(cleaned.stages[1].name, "Everything");
    assert_eq!(cleaned.stages[1].takes.iter().filter(|t| t.as_str() == "backlog").count(), 1);
    assert!(validate(&cleaned).is_empty());
}

#[test]
fn colours_are_role_names_and_the_first_builds_hue_names_still_read() {
    assert!(default_set().stages.iter().all(|s| COLOURS.contains(&s.colour.as_str())));
    for (old, role) in LEGACY_COLOURS {
        assert!(COLOURS.contains(&role), "{old} maps to a name in the palette");
        assert_eq!(canonical_colour(old), role);
    }
    let mut legacy = minimal();
    legacy.stages[0].colour = "amber".into();
    legacy.stages[1].colour = " Grey ".into();
    let cleaned = clean(legacy);
    assert_eq!((cleaned.stages[0].colour.as_str(), cleaned.stages[1].colour.as_str()), ("attention", "muted"));
    assert!(validate(&cleaned).is_empty(), "{:?}", validate(&cleaned));
}

#[test]
fn a_stored_set_with_hue_names_reads_as_role_names() {
    let mut stored = minimal();
    stored.stages[0].colour = "amber".into();
    stored.stages[1].colour = "grey".into();
    let got = parse_stored(&serde_json::to_string(&stored).unwrap()).unwrap();
    assert_eq!(got, minimal());
}

#[test]
fn an_unknown_colour_is_refused_naming_the_palette() {
    let mut set = minimal();
    set.stages[1].colour = "pink".into();
    let why = reasons(&set);
    assert!(why.contains("colour \"pink\" is not one of muted, research, info, active, attention, review, done, shipped"), "{why}");
}
