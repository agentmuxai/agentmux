// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const GOOD: &str = r#"{
  "version": 1,
  "region": "eu-west-1",
  "default_grant": "read",
  "grants": {
    "read": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["*"] },
    "deploy": {
      "tier": "write",
      "mint": { "kind": "command", "argv": ["/usr/bin/my-escalate", "--reason", "{reason}", "--duration", "{seconds}", "--session-name", "{session}"] },
      "agents": ["JCont", " other "],
      "max_mints_per_hour": 6
    }
  }
}"#;

fn err(text: &str) -> String {
    Policy::parse(text).unwrap_err()
}

fn with_grant(grant: &str) -> String {
    format!(r#"{{ "version": 1, "grants": {{ "g": {grant} }} }}"#)
}

#[test]
fn a_good_policy_reads_with_the_defaults_of_its_tier() {
    let p = Policy::parse(GOOD).unwrap();
    assert_eq!((p.region.as_str(), p.default_grant.as_deref()), ("eu-west-1", Some("read")));
    let read = &p.grants["read"];
    assert_eq!((read.tier, read.default_seconds, read.max_seconds, read.needs_reason, read.max_mints_per_hour), (Tier::Read, 3600, 3600, false, 12));
    let deploy = &p.grants["deploy"];
    assert_eq!((deploy.tier, deploy.default_seconds, deploy.needs_reason, deploy.max_mints_per_hour), (Tier::Write, 1800, true, 6), "a write grant asks for a reason");
    assert_eq!(deploy.agents, vec!["jcont", "other"], "names are lower-cased and trimmed");
    assert!(deploy.open_to("JCONT") && !deploy.open_to("lazo") && read.open_to("anyone"));
    assert_eq!(p.open_to("jcont"), vec!["deploy", "read"]);
    assert_eq!(p.open_to("lazo"), vec!["read"]);
}

#[test]
fn a_grant_can_say_it_needs_no_reason_or_is_switched_off() {
    let p = Policy::parse(&with_grant(r#"{ "tier": "write", "mint": { "kind": "host_profile", "profile": "p" }, "needs_reason": false, "disabled": true, "note": " Use CI. " }"#)).unwrap();
    let g = &p.grants["g"];
    assert!(!g.needs_reason && g.disabled);
    assert_eq!(g.note.as_deref(), Some("Use CI."));
    assert!(p.open_to("anyone").is_empty(), "a disabled grant is open to nobody, though it is still listed");
    assert_eq!(p.names(), vec!["g"]);
}

#[test]
fn nothing_is_open_to_an_agent_the_grant_does_not_name() {
    let p = Policy::parse(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" } }"#)).unwrap();
    assert!(p.grants["g"].agents.is_empty() && p.open_to("anyone").is_empty(), "closed by default");
}

#[test]
fn what_is_wrong_is_named() {
    assert!(err("{ not json").contains("line 1"), "a JSON error names the line");
    assert!(err(r#"{ "version": 2, "grants": {} }"#).contains("version 2"));
    assert!(err(r#"{ "version": 1, "grants": {}, "extra": 1 }"#).contains("extra"), "a typo is an error, not a silent default");
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "need_reason": true }"#)).contains("need_reason"));
    assert!(err(&with_grant(r#"{ "tier": "admin", "mint": { "kind": "host_profile", "profile": "p" } }"#)).contains("read or write"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "-x" } }"#)).contains("profile"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "command", "argv": [] } }"#)).contains("empty"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "command", "argv": ["{agent}"] } }"#)).contains("fixed"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "command", "argv": ["x", "{password}"] } }"#)).contains("{password}"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "command", "argv": ["x", "{reason"] } }"#)).contains("unclosed"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "default_seconds": 60 }"#)).contains("default_seconds 60"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "max_seconds": 7200 }"#)).contains("max_seconds 7200"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "default_seconds": 3600, "max_seconds": 1800 }"#)).contains("more than max_seconds"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "max_mints_per_hour": 0 }"#)).contains("disabled"));
    assert!(err(&with_grant(r#"{ "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "agents": ["a", ""] }"#)).contains("empty name"));
    assert!(err(r#"{ "version": 1, "grants": { "Bad Name": { "tier": "read", "mint": { "kind": "host_profile", "profile": "p" } } } }"#).contains("grant name"));
    assert!(err(r#"{ "version": 1, "region": "us east", "grants": {} }"#).contains("region"));
    assert!(err(r#"{ "version": 1, "default_grant": "nope", "grants": {} }"#).contains("default_grant"));
}

#[test]
fn grant_names_are_a_closed_pattern() {
    for ok in ["read", "fleet-admin-staging", "a1"] {
        assert!(is_grant_name(ok), "{ok}");
    }
    for bad in ["", "1a", "-a", "A", "a_b", "a b", "a;b", &"x".repeat(42)] {
        assert!(!is_grant_name(bad), "{bad:?}");
    }
}

#[test]
fn the_containers_config_lists_every_grant_and_holds_no_secret() {
    let p = Policy::parse(GOOD).unwrap();
    let config = p.render_config();
    assert!(config.contains("[default]\ncredential_process = agentmux-bashwrap aws-credentials --profile read\nregion = eu-west-1"), "{config}");
    assert!(config.contains("[profile read]") && config.contains("[profile deploy]\ncredential_process = agentmux-bashwrap aws-credentials --profile deploy"), "{config}");
    assert!(!config.contains("my-escalate") && !config.contains("work"), "the recipe stays on the host: {config}");
    let none = Policy::parse(r#"{ "version": 1, "grants": { "x": { "tier": "read", "mint": { "kind": "host_profile", "profile": "p" } } } }"#).unwrap();
    assert!(!none.render_config().contains("[default]"), "no default_grant, no default profile");
}

/// The cache is one per process: tests that use it take turns.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn a_missing_file_shares_nothing_and_a_bad_edit_keeps_the_last_good_version() {
    let _turn = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aws-grants.json");
    assert_eq!(load(&path).unwrap(), None, "no file: nothing shared");

    std::fs::write(&path, GOOD).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().grants.len(), 2);

    // An edit that does not parse (a different size, so the change is seen): the earlier grants stay in force.
    std::fs::write(&path, format!("{GOOD} oops")).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().grants.len(), 2, "the previous good version is kept");

    // A fix is picked up without a restart.
    std::fs::write(&path, GOOD.replace("\"max_mints_per_hour\": 6", "\"max_mints_per_hour\": 33")).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().grants["deploy"].max_mints_per_hour, 33);

    // Deleting the file withdraws everything.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(load(&path).unwrap(), None);
}

#[test]
fn a_file_that_never_parsed_is_an_error_that_names_it() {
    let _turn = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aws-grants.json");
    std::fs::write(&path, "{ \"version\": 1, \"grants\": { \"x\": { \"tier\": \"read\" } } }").unwrap();
    let e = load(&path).unwrap_err();
    assert!(e.contains("aws-grants.json") && e.contains("line 1"), "{e}");
}

#[test]
fn the_override_variable_points_at_another_file() {
    let _turn = serial();
    std::env::set_var(FILE_ENV, "/elsewhere/grants.json");
    assert_eq!(file_path(), Some(PathBuf::from("/elsewhere/grants.json")));
    std::env::remove_var(FILE_ENV);
}
