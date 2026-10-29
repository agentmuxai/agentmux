// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What runs around an agent's `git commit` and `git push` when its GitHub access goes through
//! AgentMux.
//!
//! - `git-hook <name>`: the two hooks srv installs for the agent (`pre-push`, `prepare-commit-msg`).
//!   Each runs the repository's own hook of the same name first, so a repository's husky or
//!   pre-commit setup keeps working, then does AgentMux's part: the push scan
//!   (`secret_scan.rs`), the `Agent:` and `Co-authored-by:` trailers.
//! - `git-env -- <git args>`: asked by the `git` shell function before a push or a command that
//!   makes commits. It refuses what would skip the hooks (`git push --no-verify`, a `-c
//!   core.hooksPath` of its own) and prints the environment a commit needs: the App's bot as
//!   author and committer, and who the agent works for, for the trailer.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use agentmux_common::win32::NoWindow;

use crate::github::Granted;
use crate::secret_scan::{refusal_text, scan_diff, Finding};

/// What `git <args>` needs from AgentMux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing: git runs as it is.
    Plain,
    /// A push, allowed to run its hooks.
    Push,
    /// A command that makes commits, with the directory `-C` named, if any.
    Commit { cwd: Option<String> },
}

/// Subcommands that create commits under the agent's name.
const COMMIT_COMMANDS: &[&str] = &["commit", "merge", "rebase", "cherry-pick", "am", "revert", "pull", "stash"];

/// What `git <args>` is, and whether it may run. `Err` is what to tell the agent.
pub fn plan_for(args: &[String]) -> Result<Plan, String> {
    let mut cwd = None;
    let mut own_hooks_path = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let hooks_key = |kv: &str| kv.to_ascii_lowercase().contains("core.hookspath");
        match a {
            "-c" | "--config-env" => {
                own_hooks_path |= args.get(i + 1).is_some_and(|kv| hooks_key(kv));
                i += 2;
            }
            "-C" => {
                cwd = args.get(i + 1).cloned();
                i += 2;
            }
            "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix" => i += 2,
            _ if a.starts_with("--config-env=") => {
                own_hooks_path |= hooks_key(a);
                i += 1;
            }
            _ if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let Some(sub) = args.get(i).map(String::as_str) else { return Ok(Plan::Plain) };
    let rest = &args[i + 1..];
    if sub == "push" || COMMIT_COMMANDS.contains(&sub) {
        if own_hooks_path {
            return Err("agentmux: git -c core.hooksPath is not allowed for agents: AgentMux's hooks have to run".to_string());
        }
    }
    if sub == "push" {
        if rest.iter().any(|a| is_no_verify(a)) {
            return Err("agentmux: git push --no-verify is not allowed for agents: the push scan has to run".to_string());
        }
        return Ok(Plan::Push);
    }
    if COMMIT_COMMANDS.contains(&sub) {
        return Ok(Plan::Commit { cwd });
    }
    Ok(Plan::Plain)
}

/// Whether `arg` is `--no-verify`, or an abbreviation git accepts for it (`--no-verif`, `--no-v`:
/// long options may be shortened while they stay unambiguous).
fn is_no_verify(arg: &str) -> bool {
    let name = arg.split('=').next().unwrap_or(arg);
    name.len() >= "--no-v".len() && "--no-verify".starts_with(name)
}

/// The environment a commit made for `granted` needs: the App's bot as author and committer when srv
/// said who the bot is, and the owner for the trailer when it said who the agent works for. Each is
/// on its own: a token without the bot's user id still carries the owner's credit.
pub fn commit_env(granted: &Granted) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let (Some(slug), Some(bot_id)) = (granted.bot_slug.as_deref(), granted.bot_user_id) {
        let name = format!("{slug}[bot]");
        let email = format!("{bot_id}+{slug}[bot]@users.noreply.github.com");
        env.extend([
            ("GIT_AUTHOR_NAME".to_string(), name.clone()),
            ("GIT_AUTHOR_EMAIL".to_string(), email.clone()),
            ("GIT_COMMITTER_NAME".to_string(), name),
            ("GIT_COMMITTER_EMAIL".to_string(), email),
        ]);
    }
    if let (Some(id), Some(login)) = (granted.owner_github_id, granted.owner_login.as_deref()) {
        env.push((OWNER_ID_ENV.to_string(), id.to_string()));
        env.push((OWNER_LOGIN_ENV.to_string(), login.to_string()));
    }
    env
}

/// Who the agent works for, for the commit trailer (set by `git-env`, read by the hook).
const OWNER_ID_ENV: &str = "AGENTMUX_OWNER_GITHUB_ID";
const OWNER_LOGIN_ENV: &str = "AGENTMUX_OWNER_GITHUB_LOGIN";

/// `git-env -- <git args>`: refuse, or print `KEY=VALUE` lines for the shell function to run git with.
pub fn run_git_env(args: &[String]) -> i32 {
    match plan_for(args) {
        Err(message) => {
            eprintln!("{message}");
            1
        }
        Ok(Plan::Commit { cwd }) => {
            let source = crate::github::default_repo_source(cwd.as_deref());
            if let Some((owner, repo)) = source.as_deref().and_then(crate::github::repo_of_default) {
                if let crate::github::Answer::Token(granted) = crate::github::ask(&owner, &repo) {
                    for (k, v) in commit_env(&granted) {
                        println!("{k}={v}");
                    }
                }
            }
            0
        }
        Ok(_) => 0,
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    // Windows: git is a console program, and this process has no console of its own.
    cmd.no_window();
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The hooks directory the person or the repository configured: `core.hooksPath` from the
/// repository (husky), else the person's global or system configuration. One `git config` call lists
/// every scope; the environment's own value (`command` scope: AgentMux's directory, set through
/// `GIT_CONFIG_*`) is not consulted, and an AgentMux directory found in a file is passed over.
fn configured_hooks_dir() -> Option<PathBuf> {
    hooks_dir_from(&git_output(&["config", "--type=path", "--show-scope", "--get-regexp", r"^core\.hookspath$"])?)
}

/// The winning `core.hooksPath` in `git config --show-scope --get-regexp` output
/// (`<scope>\t<key> <value>` per line, the lowest-precedence scope first).
fn hooks_dir_from(listing: &str) -> Option<PathBuf> {
    listing
        .lines()
        .filter_map(|line| {
            let (scope, rest) = line.split_once('\t')?;
            let (_, value) = rest.split_once(' ')?;
            (scope != "command" && !value.is_empty() && !value.ends_with(AGENTMUX_HOOKS_DIR)).then(|| PathBuf::from(value))
        })
        .last()
}

/// What srv calls its hooks directory (`identity/github_hooks.rs`).
pub const AGENTMUX_HOOKS_DIR: &str = "github-hooks";

/// The repository's own hook `name`, if it has one: from the configured hooks directory, else
/// `.git/hooks`.
fn repo_own_hook(name: &str) -> Option<PathBuf> {
    let dir = configured_hooks_dir().or_else(|| git_output(&["rev-parse", "--git-common-dir"]).map(|d| Path::new(&d).join("hooks")))?;
    let hook = dir.join(name);
    (hook.is_file() && is_executable(&hook)).then_some(hook)
}

/// Git skips a hook that isn't executable; so does this.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn run_script(path: &Path, args: &[String], stdin: &[u8]) -> i32 {
    let mut cmd = if cfg!(windows) {
        // Git for Windows runs hooks through its own sh; a script can't be started directly.
        let mut c = Command::new("sh");
        c.arg(path.to_string_lossy().replace('\\', "/"));
        c
    } else {
        Command::new(path)
    };
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    cmd.no_window();
    let Ok(mut child) = cmd.spawn() else {
        eprintln!("agentmux: couldn't run the repository's {} hook", path.display());
        return 1;
    };
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin);
    }
    child.wait().ok().and_then(|s| s.code()).unwrap_or(1)
}

/// The diff of what pushing `local` over `remote_sha` adds, `-U0`; `None` when git can't produce it.
fn push_diff(dir: Option<&Path>, remote: &str, local: &str, remote_sha: &str) -> Option<String> {
    let log = |range: Vec<String>| {
        let mut cmd = Command::new("git");
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        // `--cc`: a merge commit shows no patch without it, and a conflict resolution can add anything;
        // it shows what differs from every parent, so history a parent already has isn't read again.
        // `--no-textconv` and `--text`: the bytes git commits, not what a converter makes of them, and not a "Binary files differ" line for a file marked `-diff` or holding a NUL.
        cmd.args(["log", "--format=", "-p", "--cc", "-U0", "--no-color", "--no-ext-diff", "--no-textconv", "--text"]).args(range).stdin(Stdio::null()).stderr(Stdio::null());
        cmd.no_window();
        cmd.output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let is_name = !remote.is_empty() && !remote.contains([':', '/', '\\']);
    let whole_branch = || {
        // Only that remote's own tracking refs say what it has; a URL maps to none, and the other
        // remotes' history (another repository's) is not what this destination holds.
        if is_name {
            log(vec![local.to_string(), "--not".to_string(), format!("--remotes={remote}")])
        } else {
            log(vec![local.to_string()])
        }
    };
    if remote_sha.chars().all(|c| c == '0') {
        return whole_branch();
    }
    // An old commit this clone doesn't have (a force push over unfetched work): scan the whole branch.
    log(vec![format!("{remote_sha}..{local}")]).or_else(whole_branch)
}

/// `pre-push <remote> <url>`, the refs on stdin.
fn pre_push(args: &[String]) -> i32 {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    if let Some(own) = repo_own_hook("pre-push") {
        let code = run_script(&own, args, &input);
        if code != 0 {
            return code;
        }
    }
    let remote = args.first().map(String::as_str).unwrap_or("");
    let mut findings: Vec<Finding> = Vec::new();
    for line in String::from_utf8_lossy(&input).lines() {
        let mut f = line.split_whitespace();
        let (Some(_), Some(local), Some(_), Some(remote_sha)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        // A deletion adds nothing.
        if local.chars().all(|c| c == '0') {
            continue;
        }
        // A push that can't be read is refused: the scan doesn't fail open.
        let Some(diff) = push_diff(None, remote, local, remote_sha) else {
            eprintln!("agentmux: this push is refused: git couldn't list what it adds, so it can't be scanned.");
            return 1;
        };
        for found in scan_diff(&diff) {
            if !findings.contains(&found) {
                findings.push(found);
            }
        }
    }
    if findings.is_empty() {
        return 0;
    }
    eprint!("{}", refusal_text(&findings));
    1
}

/// The trailers a commit of an agent carries: who wrote it, and whose agent that is.
pub fn trailers(agent: Option<&str>, owner_id: Option<&str>, owner_login: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(agent) = agent.filter(|a| !a.is_empty()) {
        out.push(format!("Agent: {}", agent.to_ascii_lowercase()));
    }
    if let (Some(id), Some(login)) = (owner_id.filter(|v| !v.is_empty()), owner_login.filter(|v| !v.is_empty())) {
        out.push(format!("Co-authored-by: {login} <{id}+{login}@users.noreply.github.com>"));
    }
    out
}

/// `prepare-commit-msg <file> [source] [sha]`.
fn prepare_commit_msg(args: &[String]) -> i32 {
    if let Some(own) = repo_own_hook("prepare-commit-msg") {
        let code = run_script(&own, args, &[]);
        if code != 0 {
            return code;
        }
    }
    let Some(file) = args.first() else { return 0 };
    let env = |k: &str| std::env::var(k).ok();
    let lines = trailers(env("AGENTMUX_AGENT_ID").as_deref(), env(OWNER_ID_ENV).as_deref(), env(OWNER_LOGIN_ENV).as_deref());
    if lines.is_empty() {
        return 0;
    }
    let mut cmd = Command::new("git");
    cmd.args(["interpret-trailers", "--in-place", "--if-exists", "addIfDifferent"]);
    for line in &lines {
        cmd.arg("--trailer").arg(line);
    }
    cmd.arg(file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.no_window();
    // A message that can't take trailers is still a commit.
    let _ = cmd.status();
    0
}

/// Any other hook: the repository's own, with git's arguments and standard input as they came. The
/// hooks directory stands in for the repository's, so each hook the repository has must still run.
fn forward_to_repo_hook(name: &str, args: &[String]) -> i32 {
    let Some(own) = repo_own_hook(name) else { return 0 };
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("sh");
        c.arg(own.to_string_lossy().replace('\\', "/"));
        c
    } else {
        Command::new(&own)
    };
    cmd.args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    cmd.no_window();
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => {
            eprintln!("agentmux: couldn't run the repository's {name} hook");
            1
        }
    }
}

/// `git-hook <name> <git's arguments>`.
pub fn run_git_hook(name: &str, args: &[String]) -> i32 {
    match name {
        "pre-push" => pre_push(args),
        "prepare-commit-msg" => prepare_commit_msg(args),
        other => forward_to_repo_hook(other, args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_push_may_not_skip_the_hooks_and_a_commit_may_not_replace_them() {
        assert_eq!(plan_for(&args(&["push", "origin", "main"])), Ok(Plan::Push));
        assert!(plan_for(&args(&["push", "--no-verify"])).unwrap_err().contains("--no-verify"));
        assert!(plan_for(&args(&["-C", "repo", "push", "origin", "--no-verify=1"])).is_err());
        // Git accepts a long option shortened while it is unambiguous.
        for a in ["--no-verif", "--no-ver", "--no-v"] {
            assert!(plan_for(&args(&["push", a])).is_err(), "{a}");
        }
        // Other `--no-` options are fine.
        for a in ["--no-thin", "--no-force-with-lease", "--no-", "--verify"] {
            assert_eq!(plan_for(&args(&["push", a])), Ok(Plan::Push), "{a}");
        }
        assert!(plan_for(&args(&["-c", "core.hooksPath=/dev/null", "push"])).unwrap_err().contains("core.hooksPath"));
        assert!(plan_for(&args(&["-c", "Core.HooksPath=x", "commit", "-m", "x"])).is_err());
        // Another option with a value is not mistaken for the subcommand.
        assert!(plan_for(&args(&["--git-dir", "x", "push", "--no-verify"])).is_err());
        // A message that says "push --no-verify" is a message.
        assert_eq!(plan_for(&args(&["commit", "-m", "push --no-verify is refused"])), Ok(Plan::Commit { cwd: None }));
    }

    #[test]
    fn what_makes_commits_is_told_where_it_runs_and_the_rest_is_left_alone() {
        assert_eq!(plan_for(&args(&["-C", "work", "commit", "-am", "x"])), Ok(Plan::Commit { cwd: Some("work".into()) }));
        for sub in ["merge", "rebase", "cherry-pick", "am", "revert", "pull", "stash"] {
            assert!(matches!(plan_for(&args(&[sub])), Ok(Plan::Commit { .. })), "{sub}");
        }
        assert_eq!(plan_for(&args(&["status"])), Ok(Plan::Plain));
        assert_eq!(plan_for(&args(&["-c", "user.name=x", "log"])), Ok(Plan::Plain));
        assert_eq!(plan_for(&args(&[])), Ok(Plan::Plain));
    }

    #[test]
    fn the_bot_is_author_and_committer_and_the_owner_rides_along_for_the_trailer() {
        let granted = Granted {
            token: "t".into(),
            owner_github_id: Some(42),
            owner_login: Some("octo".into()),
            bot_slug: Some("muxagent".into()),
            bot_user_id: Some(77),
        };
        let env: std::collections::HashMap<_, _> = commit_env(&granted).into_iter().collect();
        assert_eq!(env["GIT_AUTHOR_NAME"], "muxagent[bot]");
        assert_eq!(env["GIT_COMMITTER_EMAIL"], "77+muxagent[bot]@users.noreply.github.com");
        assert_eq!((env[OWNER_ID_ENV].as_str(), env[OWNER_LOGIN_ENV].as_str()), ("42", "octo"));
        // Without the bot's user id the commit keeps its author, and still carries the owner.
        let no_bot: std::collections::HashMap<_, _> = commit_env(&Granted { bot_user_id: None, ..granted.clone() }).into_iter().collect();
        assert!(!no_bot.contains_key("GIT_AUTHOR_NAME"));
        assert_eq!((no_bot[OWNER_ID_ENV].as_str(), no_bot[OWNER_LOGIN_ENV].as_str()), ("42", "octo"));
        // Neither the bot nor an owner: nothing to set, and the commit stays as it is.
        assert!(commit_env(&Granted { bot_slug: None, owner_github_id: None, ..granted }).is_empty());
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args);
        cmd.no_window();
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// With a real repository: what a push would add is scanned, and what the remote already has is not.
    #[test]
    fn a_push_is_scanned_for_what_it_adds_and_not_for_what_the_remote_has() {
        let dir = std::env::temp_dir().join(format!("amx-push-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.as_path();
        git(repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "hello
").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "one"]);
        let first = git(repo, &["rev-parse", "HEAD"]);
        let token = format!("{}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        std::fs::write(repo.join("a.txt"), format!("hello
key = {token}
")).unwrap();
        git(repo, &["commit", "-q", "-am", "two"]);
        let second = git(repo, &["rev-parse", "HEAD"]);
        let zeros = "0".repeat(40);
        // A new branch: the whole history is new to the remote.
        let found = scan_diff(&push_diff(Some(repo), "origin", &second, &zeros).unwrap());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].file.as_str(), found[0].line, found[0].category), ("a.txt", 2, "GitHub token"));
        // An update over the first commit: the added line is found at its place.
        let found = scan_diff(&push_diff(Some(repo), "origin", &second, &first).unwrap());
        assert_eq!((found[0].file.as_str(), found[0].line), ("a.txt", 2));
        // The remote already has it: nothing is added.
        assert!(scan_diff(&push_diff(Some(repo), "origin", &second, &second).unwrap()).is_empty());
        // A new branch on a named remote leaves out what that remote's tracking refs hold; a URL has none
        // of its own, so history another remote holds is still scanned.
        git(repo, &["update-ref", "refs/remotes/origin/main", &second]);
        assert!(scan_diff(&push_diff(Some(repo), "origin", &second, &zeros).unwrap()).is_empty());
        let by_url = push_diff(Some(repo), "https://example.com/x/y.git", &second, &zeros).unwrap();
        assert_eq!(scan_diff(&by_url).len(), 1, "a push by URL is not excused by other remotes' history");
        // A directory that isn't a repository gives no diff, not an empty one.
        assert!(push_diff(Some(&dir.join("nowhere")), "origin", &second, &first).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A credential that only a merge commit adds (a conflict resolution) is found.
    #[test]
    fn a_credential_added_only_by_a_merge_commit_is_found() {
        let dir = std::env::temp_dir().join(format!("amx-merge-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.as_path();
        let write = |text: &str| std::fs::write(repo.join("a.txt"), format!("{text}\n")).unwrap();
        git(repo, &["init", "-q", "-b", "main"]);
        write("base");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "base"]);
        let base = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["checkout", "-q", "-b", "side"]);
        std::fs::write(repo.join("side.txt"), "side\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "side"]);
        git(repo, &["checkout", "-q", "main"]);
        write("main");
        git(repo, &["commit", "-q", "-am", "main"]);
        // The merge is stopped before its commit, and the credential goes in as part of it.
        git(repo, &["merge", "-q", "--no-commit", "--no-ff", "side"]);
        write(&format!("key = {}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8"));
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "--no-edit"]);
        let head = git(repo, &["rev-parse", "HEAD"]);
        let found = scan_diff(&push_diff(Some(repo), "origin", &head, &base).unwrap());
        assert!(found.iter().any(|f| f.file == "a.txt" && f.category == "GitHub token"), "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hooks_directory_is_the_most_specific_one_that_is_not_agentmuxs() {
        let listing = "system\tcore.hookspath /etc/hooks\nglobal\tcore.hookspath /home/a/.githooks\nlocal\tcore.hookspath .husky/_\ncommand\tcore.hookspath /x/github-hooks\n";
        assert_eq!(hooks_dir_from(listing), Some(PathBuf::from(".husky/_")));
        // A directory with a space in it; AgentMux's own directory in a file is passed over.
        let spaced = "global\tcore.hookspath /home/a b/hooks\nlocal\tcore.hookspath /r/github-hooks\n";
        assert_eq!(hooks_dir_from(spaced), Some(PathBuf::from("/home/a b/hooks")));
        assert_eq!(hooks_dir_from("command\tcore.hookspath /x/github-hooks\n"), None);
        assert_eq!(hooks_dir_from(""), None);
    }

    #[test]
    fn the_trailers_name_the_agent_and_credit_the_owner() {
        assert_eq!(
            trailers(Some("Korp"), Some("42"), Some("octo")),
            vec!["Agent: korp".to_string(), "Co-authored-by: octo <42+octo@users.noreply.github.com>".to_string()]
        );
        assert_eq!(trailers(Some("korp"), None, None), vec!["Agent: korp".to_string()]);
        assert!(trailers(None, Some("42"), None).is_empty());
    }
}
