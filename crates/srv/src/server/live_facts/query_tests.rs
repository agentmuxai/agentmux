// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use super::super::model::{ModelFacts, ModelSource};
use super::super::{AccountFacts, ContextFacts, LiveFacts, TaskFacts, TurnFacts};
use super::*;

fn model(id: Option<&str>, label: Option<&str>, source: ModelSource) -> ModelFacts {
    ModelFacts {
        id: id.map(str::to_string),
        label: label.map(str::to_string),
        source,
        why: None,
        effort: Some("high".to_string()),
        pending_switch: false,
        pending_label: None,
        definition_label: None,
        route: None,
        differs_from_definition: false,
    }
}

fn sonnet() -> ModelFacts {
    model(Some("claude-sonnet-5-5"), Some("Sonnet 5.5"), ModelSource::Effective)
}

fn cheap<'a>(model: &'a ModelFacts, state: &'a str) -> Cheap<'a> {
    Cheap { state, model, provider: "claude", container: false, account: None, seat: "" }
}

#[test]
fn a_model_filter_names_part_of_the_id_or_the_label_and_a_bang_means_not_this() {
    let (s, opus, unknown) = (sonnet(), model(Some("claude-opus-5-5"), Some("Opus 5.5"), ModelSource::Effective), model(None, None, ModelSource::Unknown));
    let only = |m: &str| Filter { model: Some(m.to_string()), ..Default::default() };
    assert!(wanted(&only("sonnet"), &cheap(&s, "idle")) && wanted(&only("SONNET 5"), &cheap(&s, "idle")));
    assert!(!wanted(&only("sonnet"), &cheap(&opus, "idle")));
    assert!(!wanted(&only("!sonnet"), &cheap(&s, "idle")) && wanted(&only("!sonnet"), &cheap(&opus, "idle")));
    assert!(wanted(&only("!sonnet"), &cheap(&unknown, "idle")), "an agent not shown to be on Sonnet is not on Sonnet for this question");
    assert!(!wanted(&only("sonnet"), &cheap(&unknown, "idle")));
}

#[test]
fn state_provider_runtime_account_and_seat_filters_each_narrow_and_all_together_and() {
    let s = sonnet();
    let mut c = cheap(&s, "working");
    c.container = true;
    c.account = Some(("stable-1", "Work account"));
    c.seat = r#"{"role":"worker","lead":"AgentY"}"#;
    let f = |g: fn(&mut Filter)| {
        let mut f = Filter::default();
        g(&mut f);
        f
    };
    assert!(wanted(&Filter::default(), &c));
    assert!(wanted(&f(|f| f.state = Some("WORKING".into())), &c) && !wanted(&f(|f| f.state = Some("idle".into())), &c));
    assert!(wanted(&f(|f| f.provider = Some("Claude".into())), &c) && !wanted(&f(|f| f.provider = Some("codex".into())), &c));
    assert!(wanted(&f(|f| f.runtime = Some("container".into())), &c) && !wanted(&f(|f| f.runtime = Some("native".into())), &c));
    assert!(wanted(&f(|f| f.account = Some("work".into())), &c) && wanted(&f(|f| f.account = Some("stable-1".into())), &c));
    assert!(!wanted(&f(|f| f.account = Some("home".into())), &c));
    assert!(wanted(&f(|f| f.seat = Some("agenty".into())), &c) && !wanted(&f(|f| f.seat = Some("agentx".into())), &c));
    assert!(!wanted(&Filter { state: Some("working".into()), provider: Some("codex".into()), ..Default::default() }, &c), "every filter must hold");
}

#[test]
fn the_seat_filter_matches_what_a_seat_says_not_its_key_names() {
    let seat = json!({ "role": "worker", "qualifier": "spawned", "lead": { "id": "agenty", "name": "AgentY" }, "area": "platform" });
    let words = seat_words(&seat);
    assert_eq!(words, "worker spawned AgentY agenty platform");
    assert_eq!(seat_words(&json!(null)), "", "no seat, no words");
    let s = sonnet();
    let mut c = cheap(&s, "idle");
    c.seat = &words;
    let by = |t: &str| Filter { seat: Some(t.into()), ..Default::default() };
    assert!(wanted(&by("platform"), &c) && wanted(&by("AGENTY"), &c) && wanted(&by("worker"), &c));
    assert!(!wanted(&by("lead"), &c) && !wanted(&by("role"), &c) && !wanted(&by("area"), &c), "a key name is not something a seat says");
}

#[test]
fn the_parameters_are_read_and_a_wrong_one_says_what_is_allowed() {
    let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
    let p = Params::from_map(&map(&[("fields", "model, turn"), ("model", "!sonnet"), ("limit", "5"), ("offset", "2"), ("host", "THIS-box")]), "this-box").unwrap();
    assert!(p.select.model && p.select.turn && !p.select.context);
    assert_eq!((p.limit, p.offset, p.filter.model.as_deref()), (Some(5), 2, Some("!sonnet")));
    assert_eq!(Params::from_map(&map(&[]), "this-box").unwrap().limit, None);
    assert!(Params::from_map(&map(&[("host", "all")]), "this-box").is_ok());
    let other = Params::from_map(&map(&[("host", "elsewhere")]), "this-box").unwrap_err();
    assert!(other.contains("not this machine") && other.contains("host argument"), "{other}");
    assert!(Params::from_map(&map(&[("host", "all")]), "this-box").unwrap().all_hosts);
    assert!(!Params::from_map(&map(&[("host", "this-box")]), "this-box").unwrap().all_hosts);
    assert!(Params::from_map(&map(&[("fields", "bogus")]), "h").unwrap_err().contains("the groups are model, runtime"));
    assert!(Params::from_map(&map(&[("limit", "many")]), "h").unwrap_err().contains("whole number"));
    assert!(Params::from_map(&map(&[("group_by", "colour")]), "h").unwrap_err().contains("group_by"));
    assert!(Params::from_map(&map(&[("detail", "everything")]), "h").unwrap_err().contains("basic, facts or full"));
}

#[test]
fn durations_read_at_a_glance() {
    assert_eq!([0, 59_000, 60_000, 240_000, 3_599_000, 3_600_000, 7_500_000].map(short_duration), ["0s", "59s", "1m", "4m", "59m", "1h 0m", "2h 5m"]);
}

#[test]
fn a_status_line_names_what_is_known_and_leaves_out_what_is_not() {
    let m = sonnet();
    let facts = LiveFacts {
        turn: Some(TurnFacts { active: true, since_ms: Some(1), duration_ms: Some(240_000), last_activity_ms: None, idle_ms: None }),
        context: Some(ContextFacts { tokens: Some(620), window: Some(1000), percent: Some(62.0), auto_compact_at: None, compacting: false, compactions: 1, observed_ms: None, stale: false }),
        account: Some(AccountFacts { stable_id: None, label: "Work".to_string() }),
        task: Some(TaskFacts { number: 1234, all: vec![1234] }),
        ..Default::default()
    };
    assert_eq!(status_line("Loap", "working", &facts, &m), "Loap | working (turn 4m) | Sonnet 5.5 high | ctx 62% | Work | #1234");
    let nothing = ModelFacts { effort: None, ..model(None, None, ModelSource::Unknown) };
    let bare = status_line("Idle", "idle", &LiveFacts::default(), &nothing);
    assert_eq!(bare, "Idle | idle | model unknown");
    let requested = ModelFacts { pending_switch: true, ..sonnet() };
    assert!(status_line("A", "idle", &LiveFacts::default(), &requested).contains("Sonnet 5.5 high?"), "a model that is not yet confirmed is marked");
}

#[test]
fn the_usage_of_an_account_reads_in_short_with_the_window_that_resets_first_first() {
    use crate::backend::account_usage::{AccountUsage, UsageStatus, UsageWindow};
    let w = |p: f64, at: u64| UsageWindow { used_percent: p, resets_at_ms: at, observed_at_ms: 0, age_ms: 0 };
    let usage = AccountUsage {
        status: UsageStatus::Reported,
        windows: [("seven_day".to_string(), w(44.4, 9)), ("five_hour".to_string(), w(60.0, 1))].into_iter().collect(),
    };
    assert_eq!(usage_short(&usage), "5h 60% 7d 44%");
}
