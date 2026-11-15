// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Path and repository-name handling for work facts: one spelling for every
//! path (forward slashes, no trailing slash), repository-relative paths so
//! two clones of the same repository compare equal, `owner/repo` from a
//! remote URL, and where bashwrap keeps an agent's live working folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// bashwrap's override for its cwd state file (`crates/bashwrap/src/bash_wrap.rs`,
/// `CWD_STATE_FILE_OVERRIDE_ENV`). When a block's env sets it, that is the file.
const CWD_STATE_FILE_OVERRIDE_ENV: &str = "AGENTMUX_BASHWRAP_CWD_STATE_FILE";

/// One spelling for a path: `\` becomes `/`, an MSYS `/c/...` becomes
/// `C:/...`, repeated slashes collapse and a trailing slash goes.
pub fn normalize_path(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/");
    // On Windows, `/c/Users/...` (Git Bash) is `C:/Users/...`.
    let b = s.as_bytes();
    if cfg!(windows) && b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/')
    {
        s = format!("{}:{}", (b[1] as char).to_ascii_uppercase(), &s[2..]);
    }
    let unc = s.starts_with("//");
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for c in s.chars() {
        if c == '/' && prev_slash {
            continue;
        }
        prev_slash = c == '/';
        out.push(c);
    }
    if unc {
        out.insert(0, '/');
    }
    // `C:/` keeps its slash; `/` alone stays `/`.
    while out.len() > 1 && out.ends_with('/') && !out.ends_with(":/") {
        out.pop();
    }
    out
}

/// Whether `raw` names an absolute path in any of the spellings agents use.
pub fn is_absolute(raw: &str) -> bool {
    let s = raw.trim().replace('\\', "/");
    let b = s.as_bytes();
    s.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Paths compare without case on Windows, where the file system does.
fn same_text(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// `path` relative to `root` (both normalized), or `None` when it isn't
/// inside it. The root itself is `""`.
pub fn relative_to(path: &str, root: &str) -> Option<String> {
    let (path, root) = (normalize_path(path), normalize_path(root));
    if root.is_empty() || path.len() < root.len() || !path.is_char_boundary(root.len()) {
        return None;
    }
    let (head, rest) = path.split_at(root.len());
    if !same_text(head, &root) {
        return None;
    }
    if rest.is_empty() {
        return Some(String::new());
    }
    if root.ends_with('/') {
        return Some(rest.to_string());
    }
    rest.strip_prefix('/').map(str::to_string)
}

/// A repository-relative path as a caller typed it: forward slashes, no
/// leading `./` or `/`, no trailing slash. `.` is the whole repository (`""`).
pub fn clean_relative(raw: &str) -> String {
    let mut s = normalize_path(raw);
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    if s == "." {
        s.clear();
    }
    s.trim_start_matches('/').to_string()
}

/// Whether the repository-relative `file` is `target` or inside it. An empty
/// target is the whole repository. A file ending in `/` is an untracked
/// folder git reported whole, which matches anything inside it too.
pub fn path_matches(file: &str, target: &str) -> bool {
    if target.is_empty() {
        return true;
    }
    let file_dir = file.strip_suffix('/');
    let file = file_dir.unwrap_or(file);
    if same_text(file, target) {
        return true;
    }
    let inside = |outer: &str, inner: &str| {
        inner.len() > outer.len()
            && inner.is_char_boundary(outer.len())
            && same_text(&inner[..outer.len()], outer)
            && inner.as_bytes()[outer.len()] == b'/'
    };
    inside(target, file) || (file_dir.is_some() && inside(file, target))
}

/// `owner/repo`, lowercase, from a remote URL in any of git's spellings
/// (`https://host/owner/repo.git`, `git@host:owner/repo`, `ssh://...`), or
/// from an `owner/repo` a caller typed. Credentials in a URL never survive:
/// only the last two path segments are kept.
pub fn normalize_repo(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    // Everything after the host: the part after `://host/`, or after the `:`
    // of an scp-style `user@host:path`.
    let path = if let Some((scheme, rest)) = s.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        rest.split_once('/').map(|(_, p)| p)?
    } else if let Some((head, rest)) = s.split_once(':') {
        if head.contains('/') || is_absolute(s) {
            return None;
        }
        rest
    } else {
        s
    };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (owner, repo) = (parts[parts.len() - 2], parts[parts.len() - 1]);
    let ok = |p: &str| p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}").to_ascii_lowercase())
}

/// The key bashwrap names an agent's cwd state file by: the block's
/// `AGENTMUX_INSTANCE_SLUG`, else its `AGENTMUX_AGENT_ID`, else the
/// registered agent id; sanitized the way bashwrap does.
fn cwd_state_key(env: &HashMap<String, String>, agent_id: &str) -> String {
    let non_empty = |k: &str| env.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let key = non_empty("AGENTMUX_INSTANCE_SLUG")
        .or_else(|| non_empty("AGENTMUX_AGENT_ID"))
        .unwrap_or(agent_id);
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Where bashwrap keeps this agent's live working folder:
/// `~/.agentmux/state/bashwrap-cwd/<key>.cwd`, unless the env overrides it.
pub fn cwd_state_file(home: &Path, env: &HashMap<String, String>, agent_id: &str) -> PathBuf {
    if let Some(over) = env.get(CWD_STATE_FILE_OVERRIDE_ENV).filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(over.trim());
    }
    home.join(".agentmux")
        .join("state")
        .join("bashwrap-cwd")
        .join(format!("{}.cwd", cwd_state_key(env, agent_id)))
}

/// The agent's working folder: the live one bashwrap recorded when it names
/// a folder that still exists, else the folder it started in (`cmd:cwd`).
pub fn working_folder(
    home: Option<&Path>,
    env: &HashMap<String, String>,
    agent_id: &str,
    start_folder: &str,
) -> Option<String> {
    let live = home
        .map(|h| cwd_state_file(h, env, agent_id))
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && Path::new(s).is_dir());
    if let Some(live) = live {
        return Some(normalize_path(&live));
    }
    let start = start_folder.trim();
    if start.is_empty() {
        return None;
    }
    let start = match (start.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest).to_string_lossy().into_owned(),
        _ => start.to_string(),
    };
    Some(normalize_path(&start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_every_spelling_of_a_path() {
        assert_eq!(normalize_path(r"C:\Users\a\repo\"), "C:/Users/a/repo");
        if cfg!(windows) {
            assert_eq!(normalize_path("/c/Users/a/repo"), "C:/Users/a/repo");
        } else {
            assert_eq!(normalize_path("/c/Users/a/repo"), "/c/Users/a/repo");
        }
        assert_eq!(normalize_path("C:/"), "C:/");
        assert_eq!(normalize_path("/home//a/repo/"), "/home/a/repo");
        assert_eq!(normalize_path(r"\\server\share\x"), "//server/share/x");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn relative_to_strips_the_root_and_rejects_outsiders() {
        assert_eq!(relative_to(r"C:\w\repo\crates\a.rs", "C:/w/repo").as_deref(), Some("crates/a.rs"));
        assert_eq!(relative_to("C:/w/repo", "C:/w/repo").as_deref(), Some(""));
        assert_eq!(relative_to("C:/w/repo2/a.rs", "C:/w/repo"), None, "a sibling with a longer name is outside");
        assert_eq!(relative_to("C:/w/other/a.rs", "C:/w/repo"), None);
        assert_eq!(relative_to("/a.rs", "/").as_deref(), Some("a.rs"));
    }

    #[cfg(windows)]
    #[test]
    fn relative_to_ignores_case_on_windows() {
        assert_eq!(relative_to("c:/W/Repo/a.rs", "C:/w/repo").as_deref(), Some("a.rs"));
    }

    #[test]
    fn cleans_a_typed_relative_path() {
        assert_eq!(clean_relative("./crates/srv/"), "crates/srv");
        assert_eq!(clean_relative(r"crates\srv\a.rs"), "crates/srv/a.rs");
        assert_eq!(clean_relative("."), "");
    }

    #[test]
    fn a_folder_matches_what_is_inside_it_and_nothing_else() {
        assert!(path_matches("crates/srv/a.rs", "crates/srv/a.rs"));
        assert!(path_matches("crates/srv/a.rs", "crates/srv"));
        assert!(path_matches("crates/srv/a.rs", ""));
        assert!(!path_matches("crates/srv2/a.rs", "crates/srv"), "a prefix is not a folder");
        assert!(!path_matches("crates", "crates/srv"));
        // An untracked folder git reported whole contains the target.
        assert!(path_matches("notes/new/", "notes/new/plan.txt"));
        assert!(path_matches("notes/new/", "notes"));
        assert!(!path_matches("notes/newer/", "notes/new"));
    }

    #[test]
    fn repo_names_come_from_every_remote_spelling() {
        for url in [
            "https://github.com/AgentMuxAI/agentmux.git",
            "https://x-access-token:secret@github.com/agentmuxai/agentmux",
            "git@github.com:agentmuxai/agentmux.git",
            "ssh://git@github.com/agentmuxai/agentmux.git",
            "agentmuxai/agentmux",
            "https://github.com/agentmuxai/agentmux/",
        ] {
            assert_eq!(normalize_repo(url).as_deref(), Some("agentmuxai/agentmux"), "{url}");
        }
        assert_eq!(normalize_repo("agentmux"), None);
        assert_eq!(normalize_repo("C:/w/repo"), None, "a local path is not a repository name");
        assert_eq!(normalize_repo(""), None);
    }

    #[test]
    fn the_cwd_state_file_follows_bashwraps_naming() {
        let home = Path::new("/h");
        let mut env = HashMap::new();
        let file = |env: &HashMap<String, String>| cwd_state_file(home, env, "AgentY");
        assert_eq!(file(&env), Path::new("/h/.agentmux/state/bashwrap-cwd/AgentY.cwd"));
        env.insert("AGENTMUX_AGENT_ID".to_string(), "Agent Y".to_string());
        assert_eq!(file(&env), Path::new("/h/.agentmux/state/bashwrap-cwd/Agent_Y.cwd"));
        env.insert("AGENTMUX_INSTANCE_SLUG".to_string(), "agenty-0629j".to_string());
        assert_eq!(file(&env), Path::new("/h/.agentmux/state/bashwrap-cwd/agenty-0629j.cwd"));
        env.insert(CWD_STATE_FILE_OVERRIDE_ENV.to_string(), "/tmp/x.cwd".to_string());
        assert_eq!(file(&env), Path::new("/tmp/x.cwd"));
    }

    #[test]
    fn the_live_folder_wins_over_the_start_folder_only_while_it_exists() {
        let home = tempfile::tempdir().unwrap();
        let live = tempfile::tempdir().unwrap();
        let env = HashMap::new();
        let state = cwd_state_file(home.path(), &env, "A");
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();

        assert_eq!(working_folder(Some(home.path()), &env, "A", "/start").as_deref(), Some("/start"));
        std::fs::write(&state, live.path().to_string_lossy().as_bytes()).unwrap();
        assert_eq!(
            working_folder(Some(home.path()), &env, "A", "/start"),
            Some(normalize_path(&live.path().to_string_lossy()))
        );
        std::fs::write(&state, "/no/such/folder/anywhere").unwrap();
        assert_eq!(working_folder(Some(home.path()), &env, "A", "/start").as_deref(), Some("/start"));
        assert_eq!(working_folder(Some(home.path()), &env, "A", ""), None);
    }
}
