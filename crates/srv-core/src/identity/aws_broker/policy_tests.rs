// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::identity::aws_broker::record;

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
fn a_file_that_cannot_be_read_is_an_error_every_time_and_not_nothing_shared() {
    let _turn = serial();
    let dir = tempfile::tempdir().unwrap();
    // A folder where the file should be: it exists, and reading it fails.
    let path = dir.path().join("aws-grants.json");
    std::fs::create_dir(&path).unwrap();
    for _ in 0..2 {
        let e = load(&path).unwrap_err();
        assert!(e.contains("can't read"), "{e}");
    }
}

#[test]
fn the_override_variable_points_at_another_file() {
    let _turn = serial();
    std::env::set_var(FILE_ENV, "/elsewhere/grants.json");
    assert_eq!(file_path(), Some(PathBuf::from("/elsewhere/grants.json")));
    std::env::remove_var(FILE_ENV);
}

/// A grants file with the one grant `deploy` for the agent `jcont`; `extra` is more grants, `, "name": {...}`.
fn one_deploy(extra: &str) -> String {
    format!(
        r#"{{ "version": 1, "grants": {{ "read": {{ "tier": "read", "mint": {{ "kind": "host_profile", "profile": "p" }}, "agents": ["*"] }},
        "deploy": {{ "tier": "write", "mint": {{ "kind": "command", "argv": ["/opt/escalate"] }}, "agents": ["jcont"] }}{extra} }} }}"#
    )
}

const NEW_ADMIN: &str = r#", "admin": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/admin"] }, "agents": ["pulse"] }"#;

#[test]
fn a_write_grant_added_to_the_file_is_held_until_it_is_allowed_and_a_removed_one_stops_at_once() {
    let _turn = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aws-grants.json");
    let record_path = record::path_beside(&path);
    std::env::set_var(FILE_ENV, &path);

    // The first read of a file there is no record of: it is the baseline, and everything in it is in force.
    std::fs::write(&path, one_deploy("")).unwrap();
    let p = current().unwrap().unwrap();
    assert!(p.grants.values().all(|g| g.hold.is_none()));
    let baseline = std::fs::read_to_string(&record_path).unwrap();
    assert!(baseline.contains("baseline"), "{baseline}");

    // The same file read again, and a stamp that changes with the same text: nothing is recorded.
    assert!(current().unwrap().unwrap().grants.values().all(|g| g.hold.is_none()));
    assert_eq!(std::fs::read_to_string(&record_path).unwrap(), baseline);

    // A write grant added to the file: recorded, and not open to the agent it names.
    std::fs::write(&path, one_deploy(NEW_ADMIN)).unwrap();
    let p = current().unwrap().unwrap();
    assert_eq!(p.grants["admin"].hold, Some(Hold::Awaiting));
    assert_eq!(p.grants["deploy"].hold, None);
    assert!(!p.open_to("pulse").contains(&"admin"), "held: not offered");
    let listing = view().unwrap();
    let admin = listing.grants.iter().find(|g| g.name == "admin").unwrap();
    assert_eq!((admin.state, admin.tier.as_str()), ("held", "write"));
    assert_eq!(listing.changes[0].kind, "added");

    // Allowed with a reason: in force from the next read, recorded under the answerer's name.
    let change = admin.change.clone();
    let event = decide(&record::Decision { grant: "admin", change: &change, allow: true, by: "loap", reason: "the test says so" }).unwrap();
    assert_eq!(event.by.as_deref(), Some("loap"));
    assert_eq!(current().unwrap().unwrap().grants["admin"].hold, None);
    let listing = view().unwrap();
    let admin = listing.grants.iter().find(|g| g.name == "admin").unwrap();
    assert_eq!((admin.state, admin.allowed_by.as_deref(), admin.allowed_reason.as_deref()), ("honoured", Some("loap"), Some("the test says so")));

    // A grant taken out of the file stops working at once.
    std::fs::write(&path, one_deploy("")).unwrap();
    assert!(!current().unwrap().unwrap().grants.contains_key("admin"));
    std::env::remove_var(FILE_ENV);
}

#[test]
fn a_record_that_cannot_be_read_holds_every_grant_and_a_new_file_is_never_held_on_a_cleared_record() {
    let _turn = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aws-grants.json");
    std::fs::write(&path, one_deploy("")).unwrap();
    // A record that is there and does not parse: every grant is held, rather than none.
    std::fs::write(record::path_beside(&path), "{ damaged").unwrap();
    let p = load(&path).unwrap().unwrap();
    assert!(p.grants.values().all(|g| g.hold == Some(Hold::Awaiting)), "{:?}", p.grants.values().map(|g| g.hold).collect::<Vec<_>>());
}

#[test]
fn a_grant_may_take_one_tag_from_the_caller_and_the_file_says_how() {
    let file = |mint: &str, tag: &str| {
        format!(r#"{{ "version": 1, "grants": {{ "g": {{ "tier": "read", "agents": ["*"], "mint": {mint}{tag} }} }} }}"#)
    };
    let command = |arg: &str| format!(r#"{{ "kind": "command", "argv": ["/opt/mint", "{arg}"] }}"#);
    let tag = |key: &str, prefix: &str| format!(r#", "caller_tag": {{ "key": "{key}", "prefix": "{prefix}" }}"#);
    let p = Policy::parse(&file(&command("{tag_key}={tag}"), &tag("Scope", "team-a/"))).unwrap();
    let spec = p.grants["g"].caller_tag.clone().unwrap();
    assert_eq!(spec, CallerTag { key: "Scope".into(), prefix: "team-a/".into() });
    assert_eq!(spec.check("g", Some("team-a/x")).unwrap(), "team-a/x");
    // A tag with no place in the command, a place with no tag, a host profile, and keys and prefixes that are not allowed.
    assert!(Policy::parse(&file(&command("--x"), &tag("Scope", "a/"))).unwrap_err().contains("no argument of the command uses {tag}"));
    assert!(Policy::parse(&file(&command("{tag}"), "")).unwrap_err().contains("has no caller_tag"));
    let host = file(r#"{ "kind": "host_profile", "profile": "work" }"#, &tag("K", ""));
    assert!(Policy::parse(&host).unwrap_err().contains("needs a command recipe"));
    for (key, prefix) in [("", "a/"), ("aws:x", "a/"), ("a b", "a/"), ("K", "a b/"), ("K", "a*")] {
        assert!(Policy::parse(&file(&command("{tag}"), &tag(key, prefix))).is_err(), "{key:?} {prefix:?}");
    }
    // No prefix is allowed: only the characters are checked.
    assert!(Policy::parse(&file(&command("{tag}"), &tag("K", ""))).is_ok());
    // A key or prefix that starts with '-' would be read as an option by the program that gets it.
    for (key, prefix) in [("-K", "a/"), ("K", "-a/")] {
        assert!(Policy::parse(&file(&command("{tag}"), &tag(key, prefix))).unwrap_err().contains("may not start with '-'"), "{key:?} {prefix:?}");
    }
}

#[test]
fn a_tag_value_that_starts_with_a_dash_is_refused_even_when_the_grant_has_no_prefix() {
    let open = CallerTag { key: "K".into(), prefix: String::new() };
    for value in ["--profile", "-x", " -x"] {
        let sentence = open.check("g", Some(value)).unwrap_err();
        assert!(sentence.contains("may not start with '-'"), "{value:?}: {sentence}");
    }
    // A dash inside is allowed.
    assert_eq!(open.check("g", Some("a-b")).unwrap(), "a-b");
}

#[test]
fn a_changed_tag_requirement_is_a_wider_recipe_and_a_grant_without_one_fingerprints_as_before() {
    let text = |prefix: &str| {
        format!(r#"{{ "version": 1, "grants": {{ "g": {{ "tier": "read", "agents": ["*"], "mint": {{ "kind": "command", "argv": ["/opt/mint", "{{tag}}"] }}, "caller_tag": {{ "key": "K", "prefix": "{prefix}" }} }} }} }}"#)
    };
    let a = record::Summary::of(&Policy::parse(&text("a/")).unwrap().grants["g"]);
    let b = record::Summary::of(&Policy::parse(&text("")).unwrap().grants["g"]);
    assert_ne!(a.recipe, b.recipe);
    assert_eq!(record::compare(&a, &b), record::Delta::Wider(vec!["recipe"]));
}
