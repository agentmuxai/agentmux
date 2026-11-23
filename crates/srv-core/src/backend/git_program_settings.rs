// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The git settings that name a program, taken from the environment for a host agent in a folder a container
//! agent can write.
//!
//! In such a folder (`mcp_folder_rule`) the folder's own `.git/config` is not the person's: a container agent
//! can write it, and a plain `git status` by the host agent runs what it names. Git reads configuration from the
//! environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`) after every file, so the
//! settings below are given there, by the same entries as `core.hooksPath` (`publish_guard`), with a value that
//! runs nothing of the folder's. A person's own value for one of them (in `~/.gitconfig`) is replaced in such a
//! folder, for these agents only; every other agent's git is left as it is.
//!
//! What is set, checked against git 2.52 (a value is read the last one wins, except `core.gitProxy`, where the
//! first wins, so that one is set by its own variable):
//!
//! - `core.fsmonitor` `false`, `core.sshCommand` `ssh`, `core.pager` `cat`, `core.editor` and `sequence.editor`
//!   `true`, `core.askPass` empty (no program asked, no fallback to `SSH_ASKPASS`), `gpg.program`,
//!   `gpg.openpgp.program` `gpg`, `gpg.ssh.program` `ssh-keygen`, `gpg.x509.program` `gpgsm`;
//! - `protocol.ext.allow` `never`: the folder's remote cannot name a command (`ext::`) to run;
//! - `credential.helper` empty, which clears every helper before it (when the environment already carries a list,
//!   as the GitHub broker's does, which starts with that empty value, it is left);
//! - `GIT_PROXY_COMMAND` empty (no proxy command), unless the environment already has a value.

use std::collections::HashMap;

use crate::backend::publish_guard::{git_config_entries, git_config_has};

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
    ("protocol.ext.allow", "never"),
];
const CREDENTIAL_HELPER: &str = "credential.helper";
/// Git reads this variable before `core.gitProxy`, and the first `core.gitProxy` it finds wins, so the
/// environment has to name it for the folder's not to be used.
const PROXY_COMMAND: &str = "GIT_PROXY_COMMAND";

/// The meta key of a shell pane of such an agent that says its git is given [`entries_over`].
pub const META_KEY_PTYSHELL_GIT_PROGRAM_SETTINGS: &str = "pty:git-program-settings";

/// The variables that give git the settings above, on top of the entries `get` already sees.
pub fn entries_over(get: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut set = |out: &mut HashMap<String, String>, key: &str, value: &str| {
        let entries = git_config_entries(|name| out.get(name).cloned().or_else(|| get(name)), key, value);
        out.extend(entries);
    };
    for (key, value) in SETTINGS {
        set(&mut out, key, value);
    }
    if !git_config_has(&get, CREDENTIAL_HELPER) {
        set(&mut out, CREDENTIAL_HELPER, "");
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
    let entries = entries_over(|k| env.get(k).cloned().or_else(|| std::env::var(k).ok()));
    env.extend(entries);
}

#[cfg(test)]
#[path = "git_program_settings_tests.rs"]
mod tests;
