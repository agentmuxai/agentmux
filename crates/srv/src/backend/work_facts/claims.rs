// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Work claims, the pure half (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md
//! §3.4): which claims touch a `WhoIsWorkingOn` question or an edited file,
//! and how a claim reads. Claims themselves are stored by
//! `storage::work_claims`; `ClaimWork` and `ReleaseWork` are
//! `server::work_facts_handlers`.

use crate::backend::storage::work_claims::WorkClaim;

use super::matching::{mentions, sort_results, words, MatchKind, Target, WhoMatch, WhoResult};
use super::overlap::EditedFile;
use super::paths::path_matches;
use super::WorkFacts;

/// How long a claim lasts when the claimer doesn't say.
pub const DEFAULT_TTL_MINUTES: u64 = 120;
/// The shortest and longest a claim may last.
pub const MIN_TTL_MINUTES: u64 = 5;
pub const MAX_TTL_MINUTES: u64 = 24 * 60;

/// A claim's lifetime in ms: the asked minutes within the bounds, else the default.
pub fn ttl_ms(minutes: Option<u64>) -> i64 {
    let m = minutes.unwrap_or(DEFAULT_TTL_MINUTES).clamp(MIN_TTL_MINUTES, MAX_TTL_MINUTES);
    (m * 60_000) as i64
}

/// Two repository-relative paths overlap when either holds the other.
fn paths_overlap(a: &str, b: &str) -> bool {
    path_matches(a, b) || path_matches(b, a)
}

/// Whether `c` was made by the caller: by UID when both have one, else by name.
pub fn is_own(c: &WorkClaim, agent: &str, uid: &str) -> bool {
    if !c.agent_uid.is_empty() && !uid.is_empty() {
        return c.agent_uid == uid;
    }
    !agent.is_empty() && c.agent.eq_ignore_ascii_case(agent)
}

/// Whether claim `c` touches the question `t`: a path either holds the
/// other, the same branch, the repository as a whole, or the query's words
/// all in the claim's topic, note or path.
pub fn claim_matches(t: &Target, c: &WorkClaim) -> bool {
    let same_repo = t.repo.is_some() && c.repo == t.repo;
    if let (Some(p), Some(cp)) = (&t.path, &c.path) {
        if same_repo && paths_overlap(cp, p) {
            return true;
        }
    }
    if let (Some(p), Some(cp)) = (&t.absolute_path, &c.absolute_path) {
        if paths_overlap(cp, p) {
            return true;
        }
    }
    let repo_only = t.path.is_none() && t.absolute_path.is_none() && t.branch.is_none() && t.query.is_none();
    if repo_only && same_repo {
        return true;
    }
    if let (Some(b), Some(cb)) = (&t.branch, &c.branch) {
        if b == cb && (t.repo.is_none() || c.repo.is_none() || same_repo) {
            return true;
        }
    }
    if let Some(q) = &t.query {
        let text = [c.topic.as_deref(), Some(c.note.as_str()), c.path.as_deref(), c.absolute_path.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if mentions(&words(q), &text) {
            return true;
        }
    }
    false
}

/// What a claim names, for a reader: `crates/srv/src/muxbus in o/r`,
/// `branch agent5/x in o/r`, `"presence"`, or several of those.
pub fn describe(c: &WorkClaim) -> String {
    let whole_repo = c.path.as_deref() == Some("");
    let mut parts: Vec<String> = Vec::new();
    if whole_repo {
        parts.push(format!("all of {}", c.repo.as_deref().unwrap_or("the repository")));
    } else if let Some(p) = c.path.as_ref().or(c.absolute_path.as_ref()) {
        parts.push(p.clone());
    }
    if let Some(b) = &c.branch {
        parts.push(format!("branch {b}"));
    }
    if let Some(t) = &c.topic {
        parts.push(format!("\"{t}\""));
    }
    // The repository is named after a path in it or a branch of it; a topic
    // claim's repository is only the one its claimer happened to be in.
    let in_repo = !whole_repo && (c.path.is_some() || c.branch.is_some());
    match &c.repo {
        Some(r) if parts.is_empty() => r.clone(),
        Some(r) if in_repo => format!("{} in {r}", parts.join(", ")),
        _ if parts.is_empty() => "(nothing named)".to_string(),
        _ => parts.join(", "),
    }
}

/// "just now", "1 minute ago", "25 minutes ago", "3 hours ago".
pub fn ago(now_ms: i64, ts_ms: i64) -> String {
    match (now_ms - ts_ms).max(0) / 60_000 {
        0 => "just now".to_string(),
        1 => "1 minute ago".to_string(),
        m if m < 120 => format!("{m} minutes ago"),
        m => format!("{} hours ago", m / 60),
    }
}

/// One claim as a `WhoIsWorkingOn` match detail:
/// `claimed crates/srv in o/r 40 minutes ago: "presence work" (expires in 80 minutes)`.
pub fn detail(c: &WorkClaim, now_ms: i64) -> String {
    let note = if c.note.trim().is_empty() { String::new() } else { format!(": \"{}\"", c.note.trim()) };
    let left = ((c.expires_at - now_ms).max(0) + 59_999) / 60_000;
    format!("claimed {} {}{note} (expires in {left} minutes)", describe(c), ago(now_ms, c.created_at))
}

/// Add every claim in `claims` that touches `t` to `results`, under the
/// agent that made it (a result of its own when its facts matched nothing),
/// then sort as `WhoIsWorkingOn` does. `facts` supplies the status, goal,
/// repository and branch of a claimer that has no result yet. The caller's
/// own claims must already be left out.
pub fn add_claim_matches(
    results: &mut Vec<WhoResult>,
    t: &Target,
    claims: &[WorkClaim],
    facts: &[WorkFacts],
    now_ms: i64,
) {
    for c in claims.iter().filter(|c| claim_matches(t, c)) {
        let same = |agent: &str, channel: &str| agent.eq_ignore_ascii_case(&c.agent) && channel == c.channel;
        let idx = results
            .iter()
            .position(|r| same(&r.agent, &r.channel))
            .or_else(|| results.iter().position(|r| r.agent.eq_ignore_ascii_case(&c.agent)));
        let idx = match idx {
            Some(i) => i,
            None => {
                let f = facts
                    .iter()
                    .find(|f| same(&f.agent, &f.channel))
                    .or_else(|| facts.iter().find(|f| f.agent.eq_ignore_ascii_case(&c.agent)));
                results.push(WhoResult {
                    agent: f.map(|f| f.agent.clone()).unwrap_or_else(|| c.agent.clone()),
                    channel: f.map(|f| f.channel.clone()).unwrap_or_else(|| c.channel.clone()),
                    status: f.map(|f| f.status.clone()).unwrap_or_else(|| "not running".to_string()),
                    goal: f.and_then(|f| f.goal.clone()),
                    repo: f.and_then(|f| f.repo.clone()).or_else(|| c.repo.clone()),
                    branch: f.and_then(|f| f.branch.clone()),
                    git_checked_ms: f.and_then(|f| f.git_checked_ms),
                    matches: Vec::new(),
                });
                results.len() - 1
            }
        };
        let r = &mut results[idx];
        r.matches.push(WhoMatch {
            kind: MatchKind::Claim,
            detail: detail(c, now_ms),
            since_ms: u64::try_from(c.created_at).ok(),
        });
        r.matches.sort_by_key(|m| m.kind);
    }
    sort_results(results);
}

/// The live claims that cover an edited file, by agents other than
/// `me_agent`: a claimed path in the same repository that holds the file
/// (or the whole repository).
pub fn covering<'a>(file: &EditedFile, me_agent: &str, claims: &'a [WorkClaim]) -> Vec<&'a WorkClaim> {
    claims
        .iter()
        .filter(|c| !c.agent.eq_ignore_ascii_case(me_agent))
        .filter(|c| c.repo.as_deref() == Some(file.repo_key.as_str()))
        .filter(|c| c.path.as_deref().is_some_and(|p| path_matches(&file.rel, p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10 * 60 * 60 * 1000;
    const MIN: i64 = 60_000;

    fn claim(agent: &str) -> WorkClaim {
        WorkClaim {
            id: format!("c-{agent}"),
            agent: agent.into(),
            channel: "stable".into(),
            repo: Some("o/r".into()),
            note: "presence work".into(),
            created_at: NOW - 40 * MIN,
            expires_at: NOW + 80 * MIN,
            ..Default::default()
        }
    }

    fn on_path(agent: &str, path: &str) -> WorkClaim {
        WorkClaim { path: Some(path.into()), ..claim(agent) }
    }

    fn target(repo: Option<&str>, path: Option<&str>, branch: Option<&str>, query: Option<&str>) -> Target {
        Target {
            repo: repo.map(Into::into),
            path: path.map(Into::into),
            branch: branch.map(Into::into),
            query: query.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn a_claimed_folder_and_a_file_in_it_match_either_way_round() {
        let folder = on_path("Agent5", "crates/srv/src/muxbus");
        assert!(claim_matches(&target(Some("o/r"), Some("crates/srv/src/muxbus/presence.rs"), None, None), &folder));
        let file = on_path("Agent5", "crates/srv/src/muxbus/presence.rs");
        assert!(claim_matches(&target(Some("o/r"), Some("crates/srv"), None, None), &file));
        assert!(!claim_matches(&target(Some("o/r"), Some("crates/mcp"), None, None), &folder));
        assert!(!claim_matches(&target(Some("o/other"), Some("crates/srv"), None, None), &folder), "another repository");
        let whole = on_path("Agent5", "");
        assert!(claim_matches(&target(Some("o/r"), Some("any/file.rs"), None, None), &whole));
    }

    #[test]
    fn branch_repository_and_topic_questions_match_their_claims() {
        let branch = WorkClaim { branch: Some("agent5/x".into()), ..claim("Agent5") };
        assert!(claim_matches(&target(Some("o/r"), None, Some("agent5/x"), None), &branch));
        assert!(claim_matches(&target(None, None, Some("agent5/x"), None), &branch));
        assert!(!claim_matches(&target(Some("o/r"), None, Some("main"), None), &branch));

        assert!(claim_matches(&target(Some("o/r"), None, None, None), &claim("Agent5")), "the repository as a whole");

        let topic = WorkClaim { topic: Some("MuxBus allowlist".into()), repo: None, ..claim("Agent5") };
        assert!(claim_matches(&target(None, None, None, Some("allowlist muxbus")), &topic));
        assert!(claim_matches(&target(None, None, None, Some("presence")), &topic), "the note counts too");
        assert!(!claim_matches(&target(None, None, None, Some("allowlist cognito")), &topic));
    }

    #[test]
    fn a_claim_reads_as_what_when_and_why() {
        assert_eq!(
            detail(&on_path("Agent5", "crates/srv/src/muxbus"), NOW),
            "claimed crates/srv/src/muxbus in o/r 40 minutes ago: \"presence work\" (expires in 80 minutes)"
        );
        let branch = WorkClaim { branch: Some("agent5/x".into()), note: String::new(), ..claim("Agent5") };
        assert_eq!(detail(&branch, NOW), "claimed branch agent5/x in o/r 40 minutes ago (expires in 80 minutes)");
        let topic = WorkClaim { topic: Some("presence".into()), repo: None, ..claim("Agent5") };
        assert!(detail(&topic, NOW).starts_with("claimed \"presence\" 40 minutes ago"));
        assert_eq!(describe(&on_path("Agent5", "")), "all of o/r");
        assert_eq!(describe(&claim("Agent5")), "o/r");
        assert_eq!(ago(NOW, NOW - 3 * 60 * MIN), "3 hours ago");
    }

    #[test]
    fn the_lifetime_defaults_to_two_hours_within_five_minutes_and_a_day() {
        assert_eq!(ttl_ms(None), 120 * MIN);
        assert_eq!(ttl_ms(Some(1)), 5 * MIN);
        assert_eq!(ttl_ms(Some(10_000)), 24 * 60 * MIN);
        assert_eq!(ttl_ms(Some(45)), 45 * MIN);
    }

    #[test]
    fn own_claims_are_known_by_uid_else_by_name() {
        let mine = WorkClaim { agent_uid: "u1".into(), ..claim("AgentY") };
        assert!(is_own(&mine, "agenty", ""));
        assert!(is_own(&mine, "someone", "u1"));
        assert!(!is_own(&mine, "AgentY", "u2"), "same name, another UID");
        assert!(!is_own(&claim("Agent5"), "AgentY", "u1"));
    }

    #[test]
    fn claims_join_the_claimers_result_or_add_one() {
        let facts = vec![WorkFacts {
            agent: "Agent4".into(),
            channel: "stable".into(),
            status: "idle".into(),
            branch: Some("agent4/web".into()),
            ..Default::default()
        }];
        let mut results = vec![WhoResult {
            agent: "Agent5".into(),
            channel: "stable".into(),
            status: "busy".into(),
            goal: None,
            repo: Some("o/r".into()),
            branch: None,
            git_checked_ms: None,
            matches: vec![WhoMatch { kind: MatchKind::Edited, detail: "recently edited".into(), since_ms: None }],
        }];
        let t = target(Some("o/r"), Some("crates/srv/src/muxbus/a.rs"), None, None);
        let claims = vec![
            on_path("Agent5", "crates/srv/src/muxbus"),
            on_path("Agent4", "crates/srv"),
            on_path("Gone", "crates"),
            on_path("Elsewhere", "frontend"),
        ];
        add_claim_matches(&mut results, &t, &claims, &facts, NOW);
        let names: Vec<&str> = results.iter().map(|r| r.agent.as_str()).collect();
        assert_eq!(names, vec!["Agent4", "Agent5", "Gone"], "claims first, then by name");
        let five = results.iter().find(|r| r.agent == "Agent5").unwrap();
        assert_eq!(five.matches.iter().map(|m| m.kind).collect::<Vec<_>>(), vec![MatchKind::Claim, MatchKind::Edited]);
        let four = results.iter().find(|r| r.agent == "Agent4").unwrap();
        assert_eq!((four.status.as_str(), four.branch.as_deref()), ("idle", Some("agent4/web")));
        assert_eq!(results.iter().find(|r| r.agent == "Gone").unwrap().status, "not running");
    }

    #[test]
    fn an_edited_file_is_covered_by_other_agents_claims_on_it_or_a_folder_above_it() {
        let file = EditedFile { repo_key: "o/r".into(), repo: "o/r".into(), rel: "crates/srv/src/muxbus/a.rs".into() };
        let claims = vec![
            on_path("Agent5", "crates/srv/src/muxbus"),
            on_path("AgentY", "crates/srv"),
            on_path("Agent4", "frontend"),
            WorkClaim { topic: Some("muxbus".into()), ..claim("Agent2") },
            WorkClaim { repo: Some("o/other".into()), ..on_path("Agent3", "crates") },
        ];
        let got: Vec<&str> = covering(&file, "agenty", &claims).iter().map(|c| c.agent.as_str()).collect();
        assert_eq!(got, vec!["Agent5"]);
    }
}
