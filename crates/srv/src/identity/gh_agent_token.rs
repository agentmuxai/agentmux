// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A GitHub token from the host's `gh-agent`, for a container agent.
//!
//! A container agent has no GitHub setup of its own, and AgentMux's own token source (the App
//! broker, `identity::github_tokens`) is not live everywhere yet. Where `gh-agent` is installed on
//! the host, it already holds the App identity the fleet works under, so `gh-agent token --repo
//! owner/repo` gives a short-lived installation token narrowed to the repository. srv runs it for
//! the agent that asked (an agent id and a repository, never a command) and hands the token over
//! the container route; nothing is written into the container.

use std::path::PathBuf;
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use serde::Deserialize;
use zeroize::Zeroizing;

/// How long `gh-agent token` may take: a cached token is instant, a fresh one is a round trip.
const TIMEOUT: Duration = Duration::from_secs(25);

/// A token `gh-agent` minted.
pub struct HostToken {
    pub token: Zeroizing<String>,
    /// Epoch seconds.
    pub expires_at: i64,
    /// The bot the token acts as, without the `[bot]` suffix (`muxagent`).
    pub bot_slug: Option<String>,
}

#[derive(Deserialize)]
struct Output {
    token: String,
    expires_at: Option<i64>,
    acting_as: Option<String>,
    /// Which narrowed token this is, as a `gh-agent` that can give one says it; a token without it is the
    /// installation-wide one.
    scope: Option<String>,
    repositories: Option<Vec<String>>,
}

/// What a container's token may do. `Api` reads contents and writes pull requests, issues and the rest; it cannot
/// write contents. `Push` writes contents and nothing else (`gh-agent token --scope`). There is no third kind: a
/// container is never handed the installation-wide token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Api,
    Push,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::Api => "api",
            Scope::Push => "push",
        }
    }
}

/// What a container is told when this machine's `gh-agent` cannot give a narrowed token (it predates `--scope`, or it
/// gave a wider one than asked): the refusal never falls back to the installation-wide token.
pub const GH_AGENT_TOO_OLD: &str = "this machine's gh-agent cannot give a container a narrowed GitHub token: update gh-agent (npm i -g @a5af/gh-agent)";

/// Whether `error` (a [`token_for`] error) is [`GH_AGENT_TOO_OLD`].
pub fn is_too_old(error: &str) -> bool {
    error == GH_AGENT_TOO_OLD
}

/// Set to `1` in a container's commands only while this machine has a publish guard (gh-agent installed); the
/// container's pre-push hook reads it (`agentmux-bashwrap`'s `publish_scan`) and asks nothing when it is absent.
pub const PUBLISH_GUARD_ENV: &str = "AGENTMUX_PUBLISH_GUARD";

/// `gh-agent` on the host's PATH, if it is installed.
pub fn installed() -> Option<PathBuf> {
    which::which("gh-agent").ok()
}

/// What an agent is told when `gh-agent` is installed but did not answer in [`TIMEOUT`]: not a missing connection.
pub fn took_too_long() -> String {
    format!("GitHub token took too long: this machine's gh-agent didn't answer in {} s. Try again in a minute", TIMEOUT.as_secs())
}

/// Whether `error` (a [`token_for`] error) is [`took_too_long`].
pub fn is_took_too_long(error: &str) -> bool {
    error.starts_with("GitHub token took too long")
}

/// A GitHub account or repository name: what the request may name, and nothing that could be read as
/// an option or a path.
pub(crate) fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with(['-', '.']) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The token in `gh-agent token --json`'s output, if it is exactly the narrowed one asked for: the scope named, and
/// the one repository. Anything else (no scope field: an older gh-agent that took `--scope` as an unknown flag and
/// gave the installation-wide token) is refused and dropped, never passed on.
fn parse(stdout: &[u8], scope: Scope, repo: &str) -> Result<HostToken, String> {
    let out: Output = serde_json::from_slice(stdout).map_err(|_| "gh-agent gave no token".to_string())?;
    if out.token.is_empty() {
        return Err("gh-agent gave no token".to_string());
    }
    let one_repo = out.repositories.as_deref().is_some_and(|r| r.len() == 1 && r[0].eq_ignore_ascii_case(repo));
    if out.scope.as_deref() != Some(scope.name()) || !one_repo {
        return Err(GH_AGENT_TOO_OLD.to_string());
    }
    let bot_slug = out.acting_as.map(|a| a.trim_end_matches("[bot]").to_string()).filter(|a| !a.is_empty());
    Ok(HostToken { token: Zeroizing::new(out.token), expires_at: out.expires_at.unwrap_or(0), bot_slug })
}

/// The numeric id in GitHub's answer for a user (`GET /users/<login>`).
fn parse_user_id(body: &[u8]) -> Option<u64> {
    #[derive(Deserialize)]
    struct User {
        id: u64,
    }
    serde_json::from_slice::<User>(body).ok().map(|u| u.id)
}

/// How long a failed lookup is not repeated: offline, or GitHub's unauthenticated rate limit, would
/// otherwise cost every token request the whole timeout.
const LOOKUP_RETRY: Duration = Duration::from_secs(60);

/// What is known of a bot's id: found for good, or failed at `Instant`.
#[derive(Clone, Copy)]
enum Lookup {
    Found(u64),
    Failed(std::time::Instant),
}

/// The answer to give without asking again, if there is one.
fn remembered(entry: Option<Lookup>, now: std::time::Instant) -> Option<Option<u64>> {
    match entry? {
        Lookup::Found(id) => Some(Some(id)),
        Lookup::Failed(at) if now.saturating_duration_since(at) < LOOKUP_RETRY => Some(None),
        Lookup::Failed(_) => None,
    }
}

async fn fetch_bot_user_id(slug: &str) -> Option<u64> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).user_agent("agentmux-srv").build().ok()?;
    let resp = client
        .get(format!("https://api.github.com/users/{slug}%5Bbot%5D"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    parse_user_id(&resp.bytes().await.ok()?)
}

/// The user id of the bot `<slug>[bot]`, which `gh-agent` doesn't say: a commit's author email is
/// `<id>+<slug>[bot]@users.noreply.github.com`. A public lookup, asked once per bot and kept; `None`
/// when GitHub can't be reached (not asked again for a minute), and then the commit keeps its own author.
pub async fn bot_user_id(slug: &str) -> Option<u64> {
    static KNOWN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Lookup>>> = std::sync::OnceLock::new();
    let known = KNOWN.get_or_init(Default::default);
    let entry = known.lock().unwrap_or_else(|e| e.into_inner()).get(slug).copied();
    if let Some(answer) = remembered(entry, std::time::Instant::now()) {
        return answer;
    }
    if !is_name(slug) {
        return None;
    }
    let id = fetch_bot_user_id(slug).await;
    let outcome = id.map_or_else(|| Lookup::Failed(std::time::Instant::now()), Lookup::Found);
    known.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.to_string(), outcome);
    id
}

/// The arguments of `gh-agent token` for a narrowed token: always with `--scope`, so no call can come back as the
/// installation-wide token. `workflows` (push only) asks for workflows write as well.
fn token_args(owner: &str, repo: &str, scope: Scope, workflows: bool) -> Vec<String> {
    let mut args = vec!["token".to_string(), "--repo".to_string(), format!("{owner}/{repo}"), "--scope".to_string(), scope.name().to_string()];
    if workflows && scope == Scope::Push {
        args.push("--workflows".to_string());
    }
    args.push("--json".to_string());
    args
}

/// A narrowed token for `owner/repo`, minted for the agent `slug`: one repository, and `scope`'s permissions only.
pub async fn token_for(slug: &str, owner: &str, repo: &str, scope: Scope, workflows: bool) -> Result<HostToken, String> {
    if !is_name(owner) || !is_name(repo) {
        return Err("not a GitHub repository".to_string());
    }
    let exe = installed().ok_or_else(|| "gh-agent isn't installed on this machine".to_string())?;
    let mut cmd = tokio::process::Command::new(&exe);
    // No instance identity or key reaches it; the agent id below is the one thing it is told.
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(token_args(owner, repo, scope, workflows))
        .env("AGENTMUX_AGENT_ID", slug)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd.no_window();
    let out = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("couldn't run gh-agent: {e}")),
        Err(_) => return Err(took_too_long()),
    };
    if !out.status.success() {
        return Err("gh-agent gave no token for that repository".to_string());
    }
    parse(&out.stdout, scope, repo)
}

/// How long `gh-agent pr merge` may take: a round trip or two to GitHub, and a merge queue's answer.
const MERGE_TIMEOUT: Duration = Duration::from_secs(90);

/// How a pull request is merged; none lets the repository's own rule (a merge queue) choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

/// What a container asked srv to merge: one pull request, by number, with the few options a merge takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub method: Option<MergeMethod>,
    pub auto: bool,
    pub delete_branch: bool,
}

/// The arguments of the host's `gh-agent pr merge`: fixed, built here from validated parts, never from the container's
/// text. There is no `--admin` and no other flag, so GitHub's own rules (a review, a merge queue, the checks) decide.
pub fn merge_args(r: &MergeRequest) -> Vec<String> {
    let mut args = vec!["pr".to_string(), "merge".to_string(), r.number.to_string(), "--repo".to_string(), format!("{}/{}", r.owner, r.repo)];
    match r.method {
        Some(MergeMethod::Squash) => args.push("--squash".to_string()),
        Some(MergeMethod::Merge) => args.push("--merge".to_string()),
        Some(MergeMethod::Rebase) => args.push("--rebase".to_string()),
        None => {}
    }
    if r.auto {
        args.push("--auto".to_string());
    }
    if r.delete_branch {
        args.push("--delete-branch".to_string());
    }
    args
}

/// Whether `r` can be run: names that cannot be read as an option or a path, and a real pull request number.
pub fn merge_request_ok(r: &MergeRequest) -> Result<(), String> {
    if !is_name(&r.owner) || !is_name(&r.repo) {
        return Err("not a GitHub repository".to_string());
    }
    if r.number == 0 || r.number > 100_000_000 {
        return Err("not a pull request number".to_string());
    }
    Ok(())
}

/// Merge `r` as the agent `slug` with the host's `gh-agent pr merge`. The token it uses stays on the host; the answer
/// is what the command said, first lines, bounded. `Ok` when the merge was done or queued, `Err` with what GitHub said.
pub async fn merge_pr(slug: &str, r: &MergeRequest) -> Result<String, String> {
    merge_request_ok(r)?;
    let exe = installed().ok_or_else(|| "gh-agent isn't installed on this machine".to_string())?;
    let mut cmd = tokio::process::Command::new(&exe);
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(merge_args(r))
        .env("AGENTMUX_AGENT_ID", slug)
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let out = match tokio::time::timeout(MERGE_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("couldn't run gh-agent: {e}")),
        Err(_) => return Err(format!("the merge took too long: gh-agent didn't answer in {} s. Look at the pull request before trying again", MERGE_TIMEOUT.as_secs())),
    };
    let said = bounded_lines(&[out.stderr.as_slice(), out.stdout.as_slice()].concat());
    if out.status.success() {
        Ok(said)
    } else if said.is_empty() {
        Err("gh-agent could not merge the pull request and said nothing".to_string())
    } else {
        Err(said)
    }
}

/// The first lines of a command's output, bounded, with the lines that only name the acting identity dropped.
fn bounded_lines(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("gh-agent:")).take(12).collect();
    lines.join("\n").chars().take(2000).collect()
}

/// How long `gh-agent scan-push` may take: the pattern list is cached for a day, but a first read, a visibility lookup and
/// the scan itself are round trips.
const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// What the host's publish guard made of a container agent's push.
#[derive(Debug, PartialEq, Eq)]
pub enum ScanOutcome {
    /// Its verdict: 0 passes, anything else refuses, with the lines to show.
    Judged { code: i64, lines: Vec<String>, public: bool },
    /// It could not judge (not run, too slow, no answer): what to tell the agent.
    Failed(String),
}

#[derive(Deserialize)]
struct Verdict {
    code: i64,
    lines: Vec<String>,
    /// Whether the repository is one the guard guards. An answer that doesn't say is taken as public.
    #[serde(default = "yes")]
    public: bool,
}

fn yes() -> bool {
    true
}

/// The verdict in `gh-agent scan-push`'s output.
fn parse_verdict(stdout: &[u8]) -> Result<(i64, Vec<String>, bool), String> {
    let v: Verdict = serde_json::from_slice(stdout).map_err(|_| "gh-agent gave no verdict".to_string())?;
    // A bounded answer: a few lines of a few hundred characters.
    let lines = v.lines.into_iter().take(50).map(|l| l.chars().take(1000).collect()).collect();
    Ok((v.code, lines, v.public))
}

/// Judge a container agent's push with the host's `gh-agent scan-push --repo owner/repo`. The `material` (JSON) goes in
/// on stdin; the arguments are fixed, there is no shell, and the working directory is the temp folder, never anything
/// of the container's. gh-agent reads no repository and runs no git for this.
pub async fn scan_push(slug: &str, repo: &str, material: &[u8]) -> ScanOutcome {
    use tokio::io::AsyncWriteExt;
    let Some((owner, name)) = repo.split_once('/') else { return ScanOutcome::Failed("not a GitHub repository".to_string()) };
    if !is_name(owner) || !is_name(name) {
        return ScanOutcome::Failed("not a GitHub repository".to_string());
    }
    let Some(exe) = installed() else { return ScanOutcome::Failed("gh-agent isn't installed on this machine".to_string()) };
    let mut cmd = tokio::process::Command::new(&exe);
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(["scan-push", "--repo", repo])
        .env("AGENTMUX_AGENT_ID", slug)
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ScanOutcome::Failed(format!("couldn't run gh-agent: {e}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let material = material.to_vec();
        // Written while the child works, so a large push can't fill the pipe and wait for a reader that isn't reading.
        tokio::spawn(async move {
            let _ = stdin.write_all(&material).await;
            let _ = stdin.shutdown().await;
        });
    }
    let out = match tokio::time::timeout(SCAN_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return ScanOutcome::Failed(format!("gh-agent failed: {e}")),
        Err(_) => {
            return ScanOutcome::Failed(format!("the publish guard didn't answer in {} s: try the push again in a minute", SCAN_TIMEOUT.as_secs()));
        }
    };
    if !out.status.success() {
        // What gh-agent said it could not do, first line, bounded.
        let said = String::from_utf8_lossy(&out.stderr);
        let first = said.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it gave no reason");
        // An older gh-agent doesn't know the command and hands it to gh.
        if said.contains("unknown command \"scan-push\"") {
            return ScanOutcome::Failed("this machine's gh-agent is too old to check a container's push: update it (npm i -g @a5af/gh-agent)".to_string());
        }
        return ScanOutcome::Failed(format!("the publish guard could not check this push: {}", first.chars().take(300).collect::<String>()));
    }
    match parse_verdict(&out.stdout) {
        Ok((code, lines, public)) => ScanOutcome::Judged { code, lines, public },
        Err(why) => ScanOutcome::Failed(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_account_and_repository_names_are_passed_on() {
        for ok in ["agentmuxai", "agentmux-landing", "my_repo.js", "a"] {
            assert!(is_name(ok), "{ok}");
        }
        for bad in ["", "-x", ".hidden", "a b", "a/b", "a;b", "--repo", "a\nb", &"x".repeat(101)] {
            assert!(!is_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_bots_user_id_is_read_from_githubs_answer() {
        assert_eq!(parse_user_id(br#"{"login":"muxagent[bot]","id":331051269,"type":"Bot"}"#), Some(331051269));
        assert_eq!(parse_user_id(br#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_user_id(b"x"), None);
    }

    #[test]
    fn a_failed_lookup_is_not_repeated_for_a_minute() {
        let now = std::time::Instant::now();
        assert_eq!(remembered(None, now), None);
        assert_eq!(remembered(Some(Lookup::Found(7)), now), Some(Some(7)));
        assert_eq!(remembered(Some(Lookup::Failed(now)), now), Some(None));
        if let Some(later) = now.checked_add(LOOKUP_RETRY + Duration::from_secs(1)) {
            assert_eq!(remembered(Some(Lookup::Failed(now)), later), None);
            assert_eq!(remembered(Some(Lookup::Found(7)), later), Some(Some(7)));
        }
    }

    #[test]
    fn the_token_and_its_expiry_are_read_from_the_json() {
        let narrowed = |extra: &str| format!(r#"{{"token":"ghs_x","scope":"api","repositories":["hello"]{extra}}}"#);
        let got = parse(narrowed(r#","expires_at":1793224607,"acting_as":"muxagent[bot]","tier":"shared","owner":"o""#).as_bytes(), Scope::Api, "hello").unwrap();
        assert_eq!((got.token.as_str(), got.expires_at, got.bot_slug.as_deref()), ("ghs_x", 1793224607, Some("muxagent")));
        assert!(parse(b"not json", Scope::Api, "hello").is_err());
        assert!(parse(br#"{"token":""}"#, Scope::Api, "hello").is_err());
        assert_eq!(parse(narrowed("").as_bytes(), Scope::Api, "HELLO").unwrap().expires_at, 0, "a repository name is not case sensitive");
    }

    #[test]
    fn a_token_that_is_not_the_narrowed_one_asked_for_is_never_passed_on() {
        // An older gh-agent takes --scope for an unknown flag and prints the installation-wide token: no scope field.
        let wide = br#"{"token":"ghs_WIDE","expires_at":1793224607,"acting_as":"muxagent[bot]","tier":"shared","owner":"o"}"#;
        assert_eq!(parse(wide, Scope::Api, "hello").err().as_deref(), Some(GH_AGENT_TOO_OLD));
        assert!(is_too_old(GH_AGENT_TOO_OLD));
        // The other scope, another repository, more than one repository, or none named.
        let api = br#"{"token":"t","scope":"api","repositories":["hello"]}"#;
        assert!(parse(api, Scope::Push, "hello").is_err(), "an api token is not a push token");
        assert!(parse(api, Scope::Api, "other").is_err());
        assert!(parse(br#"{"token":"t","scope":"api","repositories":["hello","other"]}"#, Scope::Api, "hello").is_err());
        assert!(parse(br#"{"token":"t","scope":"api"}"#, Scope::Api, "hello").is_err());
        assert!(parse(api, Scope::Api, "hello").is_ok());
    }

    fn merge(method: Option<MergeMethod>, auto: bool, delete_branch: bool) -> MergeRequest {
        MergeRequest { owner: "agentmuxai".into(), repo: "agentmux".into(), number: 42, method, auto, delete_branch }
    }

    #[test]
    fn a_merge_is_run_with_fixed_arguments_and_never_admin() {
        assert_eq!(merge_args(&merge(None, false, false)), ["pr", "merge", "42", "--repo", "agentmuxai/agentmux"]);
        assert_eq!(
            merge_args(&merge(Some(MergeMethod::Squash), true, true)),
            ["pr", "merge", "42", "--repo", "agentmuxai/agentmux", "--squash", "--auto", "--delete-branch"]
        );
        for m in [None, Some(MergeMethod::Merge), Some(MergeMethod::Rebase)] {
            let args = merge_args(&merge(m, true, true));
            assert!(!args.iter().any(|a| a == "--admin"), "{args:?}");
        }
    }

    #[test]
    fn only_a_real_repository_and_number_are_merged() {
        assert!(merge_request_ok(&merge(None, false, false)).is_ok());
        let mut bad = merge(None, false, false);
        bad.number = 0;
        assert!(merge_request_ok(&bad).is_err());
        for owner in ["", "-x", "a b", "a/b", "--admin"] {
            let mut r = merge(None, false, false);
            r.owner = owner.into();
            assert!(merge_request_ok(&r).is_err(), "{owner:?}");
        }
        let mut r = merge(None, false, false);
        r.repo = "..".into();
        assert!(merge_request_ok(&r).is_err());
    }

    #[test]
    fn what_a_merge_says_is_short_and_without_the_identity_line() {
        let said = bounded_lines(b"gh-agent: agenta as muxagent[bot] (shared App) -> o [cached]\n\n! Pull request #42 will be added to the merge queue\n");
        assert_eq!(said, "! Pull request #42 will be added to the merge queue");
        assert!(bounded_lines(&vec![b'x'; 5000]).chars().count() <= 2000);
        assert_eq!(bounded_lines(b""), "");
    }

    #[test]
    fn every_call_names_its_scope_so_none_can_come_back_installation_wide() {
        assert_eq!(token_args("o", "r", Scope::Api, false), ["token", "--repo", "o/r", "--scope", "api", "--json"]);
        assert_eq!(token_args("o", "r", Scope::Push, false), ["token", "--repo", "o/r", "--scope", "push", "--json"]);
        assert_eq!(token_args("o", "r", Scope::Push, true), ["token", "--repo", "o/r", "--scope", "push", "--workflows", "--json"]);
        assert!(!token_args("o", "r", Scope::Api, true).contains(&"--workflows".to_string()), "workflows goes only with push");
    }

    #[test]
    fn the_verdict_is_read_from_the_json_and_kept_short() {
        let (code, lines, public) = parse_verdict(br#"{"code":1,"lines":["a","b"]}"#).unwrap();
        assert_eq!((code, lines, public), (1, vec!["a".to_string(), "b".to_string()], true));
        assert!(!parse_verdict(br#"{"code":0,"lines":[],"public":false}"#).unwrap().2);
        let many = format!(r#"{{"code":0,"lines":[{}]}}"#, (0..80).map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(","));
        assert_eq!(parse_verdict(many.as_bytes()).unwrap().1.len(), 50);
        let long = format!(r#"{{"code":0,"lines":["{}"]}}"#, "x".repeat(5000));
        assert_eq!(parse_verdict(long.as_bytes()).unwrap().1[0].chars().count(), 1000);
        assert!(parse_verdict(b"not json").is_err());
        assert!(parse_verdict(br#"{"lines":[]}"#).is_err());
    }

    #[tokio::test]
    async fn only_an_owner_and_a_repository_name_reach_the_command() {
        for bad in ["", "a", "a/b/c", "-x/y", "a/b;c", "../x/y", "a/--flag"] {
            assert!(matches!(scan_push("aria", bad, b"{}").await, ScanOutcome::Failed(_)), "{bad:?}");
        }
    }
}
