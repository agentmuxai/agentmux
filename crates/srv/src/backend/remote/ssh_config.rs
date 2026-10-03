// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The hosts the user's `~/.ssh/config` names, for the connection picker
//! (P2 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! Only names are read, never anything else in the file: a `Host` alias that
//! is a plain name (no `*`, `?` or `!` pattern) is a connection the user can
//! open, and `ssh` itself resolves everything about it. `Include` is followed
//! as `ssh` does, relative to `~/.ssh`, with a `*` in the last part of the
//! path (`Include config.d/*`), the common form.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How deep `Include` is followed (ssh's own limit is 16; real configs use one
/// or two levels).
const MAX_INCLUDE_DEPTH: usize = 8;

/// The plain `Host` names in the user's ssh config, sorted, each once.
pub fn hosts() -> Vec<String> {
    let Some(ssh_dir) = dirs::home_dir().map(|h| h.join(".ssh")) else {
        return Vec::new();
    };
    hosts_in(&ssh_dir.join("config"), &ssh_dir)
}

/// [`hosts`] for a given config file, with `Include` relative to `ssh_dir`.
pub fn hosts_in(config: &Path, ssh_dir: &Path) -> Vec<String> {
    let mut found = BTreeSet::new();
    read(config, ssh_dir, 0, &mut found);
    found.into_iter().collect()
}

fn read(path: &Path, ssh_dir: &Path, depth: usize, found: &mut BTreeSet<String>) {
    if depth > MAX_INCLUDE_DEPTH {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let Some((keyword, args)) = split_line(line) else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            for name in args {
                if is_plain_name(&name) {
                    found.insert(name);
                }
            }
        } else if keyword.eq_ignore_ascii_case("include") {
            for pattern in args {
                for file in expand_include(&pattern, ssh_dir) {
                    read(&file, ssh_dir, depth + 1, found);
                }
            }
        }
    }
}

/// A config line's keyword and its arguments: `Keyword arg ...` or
/// `Keyword=arg ...`, with double-quoted arguments kept whole. `None` for a
/// blank line or a comment.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let split_at = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let keyword = line[..split_at].to_string();
    let rest = line[split_at..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    Some((keyword, args))
}

/// A name the user can open as it is: not a pattern, and nothing ssh's own
/// destination check (conn.rs) would refuse.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['*', '?', '!'])
        && super::conn::ConnTarget::parse(name).is_ok_and(|t| !t.is_local())
}

/// The files an `Include` names: `~/` is the home, a relative path is under
/// `ssh_dir`, and a `*` in the last part matches any run of characters there.
fn expand_include(pattern: &str, ssh_dir: &Path) -> Vec<PathBuf> {
    let path = match pattern.strip_prefix("~/") {
        Some(rest) => match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => return Vec::new(),
        },
        None if Path::new(pattern).is_absolute() => PathBuf::from(pattern),
        None => ssh_dir.join(pattern),
    };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let Some((prefix, suffix)) = name.split_once('*') else {
        return vec![path];
    };
    if suffix.contains('*') {
        return Vec::new();
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| {
                n.len() >= prefix.len() + suffix.len()
                    && n.starts_with(prefix)
                    && n.ends_with(suffix)
            })
        })
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn plain_host_names_are_listed_and_patterns_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let config = write(
            dir.path(),
            "config",
            "# a comment\n\
             Host area54 starpower\n\
             \x20 HostName 10.0.0.5\n\
             Host=prod/web\n\
             host db+replica \"two words\"\n\
             Host *\n\
             Host *.example.com !bastion ?x\n\
             Host -oProxyCommand=calc\n\
             Match host foo\n",
        );
        assert_eq!(
            hosts_in(&config, dir.path()),
            ["area54", "db+replica", "prod/web", "starpower"]
        );
    }

    #[test]
    fn includes_are_followed_relative_to_the_ssh_dir() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "config.d/work", "Host work-box\n");
        write(
            dir.path(),
            "config.d/home.conf",
            "Host nas\nInclude nested\n",
        );
        write(dir.path(), "nested", "Host deep\n");
        write(dir.path(), "other", "Host other\n");
        let config = write(dir.path(), "config", "Include config.d/*\nHost top\n");
        assert_eq!(
            hosts_in(&config, dir.path()),
            ["deep", "nas", "top", "work-box"]
        );

        let config = write(dir.path(), "config2", "Include config.d/*.conf other\n");
        assert_eq!(hosts_in(&config, dir.path()), ["deep", "nas", "other"]);
    }

    #[test]
    fn an_include_loop_ends() {
        let dir = tempfile::tempdir().unwrap();
        let config = write(dir.path(), "config", "Host a\nInclude config\n");
        assert_eq!(hosts_in(&config, dir.path()), ["a"]);
    }

    #[test]
    fn no_config_is_no_hosts() {
        let dir = tempfile::tempdir().unwrap();
        assert!(hosts_in(&dir.path().join("config"), dir.path()).is_empty());
    }
}
