// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::mcp_config_file::with_data_home;
use serde_json::json;
use std::path::Path;

fn none(_: &str) -> Option<String> {
    None
}

fn as_env(entries: Vec<(String, String)>) -> HashMap<String, String> {
    entries.into_iter().collect()
}

/// The (key, value) pairs the `GIT_CONFIG_*` entries of `env` name, in order (a slot the process environment fills
/// is not in `env`).
fn pairs(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let count: usize = env.get("GIT_CONFIG_COUNT").and_then(|c| c.parse().ok()).unwrap_or(0);
    (0..count).filter_map(|i| Some((env.get(&format!("GIT_CONFIG_KEY_{i}"))?.clone(), env.get(&format!("GIT_CONFIG_VALUE_{i}"))?.clone()))).collect()
}

fn value_of(env: &HashMap<String, String>, key: &str) -> Option<String> {
    pairs(env).into_iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn pane(mode: &str, provider: &str, cwd: &Path) -> crate::backend::obj::MetaMapType {
    [("agentMode", json!(mode)), ("agentProvider", json!(provider)), ("cmd:cwd", json!(cwd.to_string_lossy()))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[test]
fn every_program_setting_is_given_a_value_that_runs_nothing_of_the_folders() {
    let env = as_env(entries_over(none));
    for (key, value) in SETTINGS {
        assert_eq!(value_of(&env, key).as_deref(), Some(*value), "{key}");
    }
    assert_eq!(value_of(&env, "credential.helper").as_deref(), Some(""), "the helpers before it are cleared");
    assert_eq!(env.get("GIT_PROXY_COMMAND").map(String::as_str), Some(""));
}

#[test]
fn what_the_environment_already_carries_is_kept_beside_them() {
    let carried = HashMap::from([
        ("GIT_CONFIG_COUNT".to_string(), "3".to_string()),
        ("GIT_CONFIG_KEY_0".to_string(), "core.hooksPath".to_string()),
        ("GIT_CONFIG_VALUE_0".to_string(), "/hooks".to_string()),
        ("GIT_CONFIG_KEY_1".to_string(), "credential.helper".to_string()),
        ("GIT_CONFIG_VALUE_1".to_string(), String::new()),
        ("GIT_CONFIG_KEY_2".to_string(), "credential.helper".to_string()),
        ("GIT_CONFIG_VALUE_2".to_string(), "broker".to_string()),
        ("GIT_PROXY_COMMAND".to_string(), "the-persons-proxy".to_string()),
    ]);
    let mut env = carried.clone();
    env.extend(entries_over(|k| carried.get(k).cloned()));
    let all = pairs(&env);
    assert_eq!(&all[..3], &[("core.hooksPath".into(), "/hooks".into()), ("credential.helper".into(), String::new()), ("credential.helper".into(), "broker".into())]);
    assert_eq!(all.iter().filter(|(k, _)| k == "credential.helper").count(), 2, "a list that is already there is not added to");
    assert_eq!(all.len(), 3 + SETTINGS.len());
    assert_eq!(env.get("GIT_PROXY_COMMAND").map(String::as_str), Some("the-persons-proxy"));
    let mut own = carried.clone();
    own.insert("GIT_CONFIG_KEY_1".into(), "core.editor".into());
    own.insert("GIT_CONFIG_VALUE_1".into(), "vim".into());
    own.extend(entries_over(|k| own.get(k).cloned()));
    assert_eq!(value_of(&own, "core.editor").as_deref(), Some("true"), "over the agent's own value");
}

#[test]
fn only_a_host_agent_in_such_a_folder_is_given_them_whatever_its_harness() {
    let (data, folder, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    with_data_home(data.path(), || {
        crate::backend::mcp_folder_rule::mark(&folder.path().to_string_lossy());
        let env_of = |meta: &crate::backend::obj::MetaMapType| {
            let mut env = HashMap::from([("KEPT".to_string(), "yes".to_string())]);
            apply_for_spawn(&mut env, meta);
            env
        };
        for provider in ["claude", "codex", "gemini"] {
            let there = env_of(&pane("host", provider, folder.path()));
            assert_eq!(value_of(&there, "core.fsmonitor").as_deref(), Some("false"), "{provider}");
            assert_eq!(there.get("KEPT").map(String::as_str), Some("yes"));
        }
        for (why, meta) in [
            ("a folder no container agent has to do with", pane("host", "claude", other.path())),
            ("the container agent itself", pane("container", "claude", folder.path())),
        ] {
            assert_eq!(env_of(&meta), HashMap::from([("KEPT".to_string(), "yes".to_string())]), "{why}: git left as it is");
        }
    });
}

/// Runs git in `repo` with none of the person's own configuration, and `env` on top.
fn run_git(repo: &Path, home: &Path, args: &[&str], env: &HashMap<String, String>) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_PROXY_COMMAND")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_EXTERNAL_DIFF")
        .envs(env)
        .output()
        .expect("git runs")
}

/// A repository whose own `.git/config` names a marker program for every setting that runs one, and a remote
/// that names a command (`ext::`), one that goes through the proxy and one through ssh.
struct Folder {
    dir: tempfile::TempDir,
    marks: std::path::PathBuf,
}

impl Folder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let marks = dir.path().join("marks");
        std::fs::create_dir_all(&marks).unwrap();
        let script = dir.path().join("mark.sh");
        std::fs::write(&script, format!("#!/bin/sh\necho \"$@\" >> '{}/'\"$1\"\nexit 1\n", slashes(&marks))).unwrap();
        let folder = Folder { dir, marks };
        let (repo, remote) = (folder.repo(), folder.dir.path().join("remote.git"));
        for args in [vec!["init", "-q", "."], vec!["config", "user.email", "t@example.invalid"], vec!["config", "user.name", "t"]] {
            folder.git(&args, &HashMap::new());
        }
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        folder.git(&["add", "a.txt"], &HashMap::new());
        folder.git(&["commit", "-q", "-m", "one"], &HashMap::new());
        folder.git(&["init", "-q", "--bare", &slashes(&remote)], &HashMap::new());
        let mark = |name: &str| format!("sh {} {name}", slashes(&script));
        for (key, value) in [
            ("core.fsmonitor", mark("fsmonitor")),
            ("core.sshCommand", mark("ssh")),
            ("core.gitProxy", mark("proxy")),
            ("core.pager", mark("pager")),
            ("core.editor", mark("editor")),
            ("core.askPass", mark("askpass")),
            ("gpg.program", mark("gpg")),
            ("gpg.ssh.program", mark("gpgssh")),
            ("credential.helper", format!("!{}", mark("credential"))),
            ("sequence.editor", mark("sequence")),
            ("protocol.ext.allow", "always".to_string()),
            ("remote.origin.url", slashes(&remote)),
            ("remote.viaext.url", format!("ext::sh {} ext %S", slashes(&script))),
            ("remote.viassh.url", "ssh://127.0.0.1:1/x".to_string()),
            ("remote.viagit.url", "git://127.0.0.1:9/x".to_string()),
        ] {
            folder.git(&["config", key, &value], &HashMap::new());
        }
        folder
    }

    fn repo(&self) -> std::path::PathBuf {
        self.dir.path().join("repo")
    }

    /// Runs git in the folder; nothing of the person's own configuration is read. What it printed.
    fn git(&self, args: &[&str], env: &HashMap<String, String>) -> String {
        let repo = self.repo();
        std::fs::create_dir_all(&repo).unwrap();
        let out = run_git(&repo, self.dir.path(), args, env);
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    }

    /// The marker programs that ran, by name.
    fn ran(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.marks).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn forget(&self) {
        for entry in std::fs::read_dir(&self.marks).unwrap().flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn git_in_the_folder_runs_nothing_its_own_config_names() {
    let folder = Folder::new();
    folder.forget();
    let given = as_env(entries_over(none));
    // Without the settings the folder's config does run its programs: the check is not empty.
    for args in [vec!["status"], vec!["fetch", "viaext"], vec!["ls-remote", "viassh"]] {
        folder.git(&args, &HashMap::new());
    }
    assert_eq!(folder.ran(), vec!["ext", "fsmonitor", "ssh"], "what the folder's config runs by itself");
    let proxied = folder.git(&["ls-remote", "viagit"], &HashMap::new());
    assert!(proxied.contains("cannot start proxy"), "and the proxy command it names is started: {proxied}");
    folder.forget();
    for args in [
        vec!["status"],
        vec!["log"],
        vec!["diff"],
        vec!["fetch", "origin"],
        vec!["fetch", "viaext"],
        vec!["ls-remote", "viassh"],
    ] {
        folder.git(&args, &given);
    }
    assert_eq!(folder.ran(), Vec::<String>::new(), "with them, none of it runs");
    let direct = folder.git(&["ls-remote", "viagit"], &given);
    assert!(!direct.contains("proxy"), "no proxy command is started: {direct}");
    // And git still works there: the history and the file remote are read.
    let out = run_git(&folder.repo(), folder.dir.path(), &["log", "--oneline"], &given);
    assert!(String::from_utf8_lossy(&out.stdout).contains("one"), "{out:?}");
}

#[test]
fn the_settings_are_what_git_reads_for_the_programs_a_person_is_not_present_for() {
    let folder = Folder::new();
    let given = as_env(entries_over(none));
    let read = |args: &[&str]| String::from_utf8_lossy(&run_git(&folder.repo(), folder.dir.path(), args, &given).stdout).trim().to_string();
    assert_eq!(read(&["var", "GIT_PAGER"]), "cat", "the pager");
    assert_eq!(read(&["var", "GIT_EDITOR"]), "true", "the editor");
    for (key, value) in SETTINGS {
        assert_eq!(read(&["config", "--get", key]), *value, "{key}");
    }
    let helpers = run_git(&folder.repo(), folder.dir.path(), &["config", "--get-all", "credential.helper"], &given);
    let helpers = String::from_utf8_lossy(&helpers.stdout).replace("\r\n", "\n");
    assert_eq!(helpers.strip_suffix('\n').unwrap_or(&helpers).rsplit('\n').next(), Some(""), "the last helper is the empty one, which clears those before it: {helpers:?}");
}
