// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The git settings that name a program, taken from the environment for a host agent in a folder a container
//! agent can write.
//!
//! In such a folder (`mcp_folder_rule`) the folder's own `.git/config` is not the person's: a container agent
//! can write it, and a plain `git status` by the host agent runs what it names. Git reads configuration from the
//! environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`) after every file, so the
//! settings below are given there, by the same entries as `core.hooksPath` (`publish_guard`), with a value that
//! runs nothing of the folder's. For the settings a person chooses for themselves (their ssh command, their
//! signing programs, their editors) the value is the one git gives from the person's system and global
//! configuration alone, and the neutral value is used only when neither sets one: the folder's value is never
//! used. Every other agent's git is left as it is.
//!
//! What is set, checked against git 2.52 (a value is read the last one wins, except `core.gitProxy`, where the
//! first wins, so that one is set by its own variable):
//!
//! - `core.sshCommand` (`ssh`), `core.editor` and `sequence.editor` (`true`), `gpg.program` and
//!   `gpg.openpgp.program` (`gpg`), `gpg.ssh.program` (`ssh-keygen`), `gpg.x509.program` (`gpgsm`), `gpg.ssh.defaultKeyCommand` (`true`): the person's
//!   own value, else the one in brackets;
//! - `core.fsmonitor` `false`, `core.pager` `cat`, `pager.<command>` `false` for every git command (a pager the
//!   folder names for one command, such as `pager.log`, takes precedence over `core.pager`), `core.askPass` empty (no program asked, no fallback to
//!   `SSH_ASKPASS`);
//! - `protocol.ext.allow` `never`: the folder's remote cannot name a command (`ext::`) to run;
//! - `credential.helper` empty, first of the entries, which clears every helper the files named, so the helpers the
//!   environment lists after it (the GitHub broker's, under `credential.<url>.helper`) are the only ones; a
//!   `credential.helper` the environment itself lists is left;
//! - `GIT_PROXY_COMMAND` empty (no proxy command), unless the environment already has a value.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::win32::NoWindow;

use crate::backend::publish_guard::{git_config_entries, git_config_has_value, git_config_prepend};

/// The settings that name a program, and the value that runs none of the folder's.
const SETTINGS: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.sshCommand", "ssh"),
    ("core.pager", "cat"),
    ("core.editor", "true"),
    ("sequence.editor", "true"),
    ("core.askPass", ""),
    ("gpg.program", "gpg"),
    ("gpg.openpgp.program", "gpg"),
    ("gpg.ssh.program", "ssh-keygen"),
    ("gpg.x509.program", "gpgsm"),
    ("gpg.ssh.defaultKeyCommand", "true"),
    ("protocol.ext.allow", "never"),
];
/// The commands of git 2.52 (`git --list-cmds=builtins`): `pager.<command>` is `false` for each, since a folder's own
/// `pager.<command>` is read before `core.pager` and `GIT_PAGER`.
const PAGED_COMMANDS: &[&str] = &[
    "add", "am", "annotate", "apply", "archive", "backfill", "bisect", "blame", "branch", "bugreport", "bundle",
    "cat-file", "check-attr", "check-ignore", "check-mailmap", "check-ref-format", "checkout",
    "checkout--worker", "checkout-index", "cherry", "cherry-pick", "clean", "clone", "column", "commit",
    "commit-graph", "commit-tree", "config", "count-objects", "credential", "credential-cache",
    "credential-cache--daemon", "credential-store", "describe", "diagnose", "diff", "diff-files", "diff-index",
    "diff-pairs", "diff-tree", "difftool", "fast-export", "fast-import", "fetch", "fetch-pack", "fmt-merge-msg",
    "for-each-ref", "for-each-repo", "format-patch", "fsck", "fsck-objects", "fsmonitor--daemon", "gc",
    "get-tar-commit-id", "grep", "hash-object", "help", "hook", "index-pack", "init", "init-db",
    "interpret-trailers", "last-modified", "log", "ls-files", "ls-remote", "ls-tree", "mailinfo", "mailsplit",
    "maintenance", "merge", "merge-base", "merge-file", "merge-index", "merge-ours", "merge-recursive",
    "merge-recursive-ours", "merge-recursive-theirs", "merge-subtree", "merge-tree", "mktag", "mktree",
    "multi-pack-index", "mv", "name-rev", "notes", "pack-objects", "pack-redundant", "pack-refs", "patch-id",
    "pickaxe", "prune", "prune-packed", "pull", "push", "range-diff", "read-tree", "rebase", "receive-pack",
    "reflog", "refs", "remote", "remote-ext", "remote-fd", "repack", "replace", "replay", "repo", "rerere",
    "reset", "restore", "rev-list", "rev-parse", "revert", "rm", "send-pack", "shortlog", "show", "show-branch",
    "show-index", "show-ref", "sparse-checkout", "stage", "stash", "status", "stripspace", "submodule--helper",
    "survey", "switch", "symbolic-ref", "tag", "unpack-file", "unpack-objects", "update-index", "update-ref",
    "update-server-info", "upload-archive", "upload-archive--writer", "upload-pack", "var", "verify-commit",
    "verify-pack", "verify-tag", "version", "whatchanged", "worktree", "write-tree"
];
/// The settings of [`SETTINGS`] a person chooses for themselves: their own value, from the system and global
/// configuration, is kept.
const PERSONS: &[&str] = &["core.sshCommand", "core.editor", "sequence.editor", "gpg.program", "gpg.openpgp.program", "gpg.ssh.program", "gpg.x509.program", "gpg.ssh.defaultKeyCommand"];
/// The names of [`PERSONS`] as git matches them (section and name in lower case).
const PERSONS_PATTERN: &str = r"^(core\.sshcommand|core\.editor|sequence\.editor|gpg\.program|gpg\.(openpgp|ssh|x509)\.program|gpg\.ssh\.defaultkeycommand)$";
/// The key under which [`persons_values`] holds the OpenPGP program git uses: `gpg.program` is the legacy name of
/// `gpg.openpgp.program`, so the one set last, of the two, is the person's.
const OPENPGP_EFFECTIVE: &str = "gpg.(openpgp)";
/// How long what was read from the person's configuration is kept.
const PERSONS_KEPT: Duration = Duration::from_secs(30);
const CREDENTIAL_HELPER: &str = "credential.helper";
/// Git reads this variable before `core.gitProxy`, and the first `core.gitProxy` it finds wins, so the
/// environment has to name it for the folder's not to be used.
const PROXY_COMMAND: &str = "GIT_PROXY_COMMAND";

/// The meta key of a shell pane of such an agent that says its git is given [`entries_over`].
pub const META_KEY_PTYSHELL_GIT_PROGRAM_SETTINGS: &str = "pty:git-program-settings";

/// The person's own value (lower-cased name to value) for each of [`PERSONS`] that their system or global git
/// configuration sets, read outside any repository: what git gives with no folder's configuration. Kept for a
/// short while, so a start of several agents asks git once.
pub fn persons_values() -> HashMap<String, String> {
    static KEPT: Mutex<Option<(Instant, HashMap<String, String>)>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, values)) = kept.as_ref() {
        if at.elapsed() < PERSONS_KEPT {
            return values.clone();
        }
    }
    let values = read_persons(None);
    *kept = Some((Instant::now(), values.clone()));
    values
}

/// [`persons_values`] read now; `home` names the person's home folder when it is not this process's.
fn read_persons(home: Option<&Path>) -> HashMap<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["config", "--show-scope", "--get-regexp", PERSONS_PATTERN]);
    cmd.current_dir(home.map_or_else(std::env::temp_dir, Path::to_path_buf)).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
    if let Some(home) = home {
        cmd.env("HOME", home).env("USERPROFILE", home);
    }
    cmd.no_window();
    crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
    match cmd.output() {
        Ok(out) if out.status.success() => parse_scoped(&String::from_utf8_lossy(&out.stdout)),
        _ => HashMap::new(),
    }
}

/// The values of `git config --show-scope --get-regexp` lines (`<scope>\t<name> <value>`) that the system or the
/// global configuration gave, the later one over the earlier.
fn parse_scoped(output: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in output.lines() {
        let Some((scope, rest)) = line.split_once('\t') else { continue };
        if scope != "system" && scope != "global" {
            continue;
        }
        let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
        let (name, value) = (name.to_ascii_lowercase(), value.trim_end_matches('\r').to_string());
        if name == "gpg.program" || name == "gpg.openpgp.program" {
            out.insert(OPENPGP_EFFECTIVE.to_string(), value.clone());
        }
        out.insert(name, value);
    }
    out
}

/// The variables that give git the settings above, on top of the entries `get` already sees; `own` is
/// [`persons_values`].
pub fn entries_over(get: impl Fn(&str) -> Option<String>, own: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out: HashMap<String, String> = HashMap::new();
    if !git_config_has_value(&get, CREDENTIAL_HELPER, "") {
        out.extend(git_config_prepend(&get, CREDENTIAL_HELPER, ""));
    }
    let mut set = |out: &mut HashMap<String, String>, key: &str, value: &str| {
        let entries = git_config_entries(|name| out.get(name).cloned().or_else(|| get(name)), key, value);
        out.extend(entries);
    };
    for (key, value) in SETTINGS {
        let lookup = if matches!(*key, "gpg.program" | "gpg.openpgp.program") { OPENPGP_EFFECTIVE.to_string() } else { key.to_ascii_lowercase() };
        let kept = PERSONS.contains(key).then(|| own.get(&lookup)).flatten();
        set(&mut out, key, kept.map_or(*value, String::as_str));
    }
    for command in PAGED_COMMANDS {
        set(&mut out, &format!("pager.{command}"), "false");
    }
    if get(PROXY_COMMAND).is_none() {
        out.insert(PROXY_COMMAND.to_string(), String::new());
    }
    out.into_iter().collect()
}

/// The spawn environment of an agent whose pane has `block_meta`: given the settings when it is a host agent in a
/// folder the rule holds for, and left as it is otherwise.
pub fn apply_for_spawn(env: &mut HashMap<String, String>, block_meta: &crate::backend::obj::MetaMapType) {
    if crate::backend::mcp_folder_rule::folder_of(block_meta).is_none() {
        return;
    }
    let entries = entries_over(|k| env.get(k).cloned().or_else(|| std::env::var(k).ok()), &persons_values());
    env.extend(entries);
}

#[cfg(test)]
#[path = "git_program_settings_tests.rs"]
mod tests;
