// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The git hooks directory of an agent whose GitHub access goes through AgentMux.
//!
//! srv writes a script for every client-side hook and points the agent's `core.hooksPath` at the
//! directory (`identity::github_tokens::apply_agent_git_in`). Git looks for each hook there and
//! nowhere else, so the directory has to stand in for the repository's own (husky, the
//! pre-commit framework, commitlint, git-lfs): every script only starts `agentmux-bashwrap
//! git-hook`, which runs the repository's own hook of the same name and, for `pre-push` (the
//! secret scan) and `prepare-commit-msg` (the `Agent:` and owner trailers), does AgentMux's part.
//! The logic is Rust (`crates/bashwrap/src/git_hooks.rs`), so the scripts never need to change.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The hooks AgentMux installs: the client-side ones git runs, each forwarded to the repository's own.
pub const HOOK_NAMES: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "reference-transaction",
    "post-index-change",
    "sendemail-validate",
];

/// What the directory is called under the AgentMux root.
const DIR_NAME: &str = "github-hooks";

/// A hook's script: git runs hooks through sh, on Windows too.
pub fn hook_script(name: &str) -> String {
    format!("#!/bin/sh\n# Written by AgentMux for agents whose GitHub access goes through it.\nexec agentmux-bashwrap git-hook {name} \"$@\"\n")
}

/// Write the hooks under `root` (what is already right is left alone) and return their directory.
pub fn ensure_hooks_dir(root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    for name in HOOK_NAMES {
        let path = dir.join(name);
        let script = hook_script(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
            std::fs::write(&path, script)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(dir)
}

/// This install's hooks directory, written the first time it is asked for.
pub fn hooks_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = agentmux_common::data_paths::agentmux_root().ok()?;
        match ensure_hooks_dir(&root) {
            Ok(dir) => Some(dir),
            Err(e) => {
                tracing::warn!(target: "identity", error = %e, "identity.github: couldn't write the agents' git hooks; pushes won't be scanned");
                None
            }
        }
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hooks_are_written_once_and_start_bashwrap_with_their_own_name() {
        // The ones a repository's tooling relies on are there, not only the two AgentMux has work in.
        for name in ["pre-commit", "commit-msg", "post-checkout", "post-merge", "post-commit", "pre-push", "prepare-commit-msg"] {
            assert!(HOOK_NAMES.contains(&name), "{name}");
        }
        let root = tempfile::tempdir().unwrap();
        let dir = ensure_hooks_dir(root.path()).unwrap();
        for name in HOOK_NAMES {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            assert!(text.starts_with("#!/bin/sh\n"));
            assert!(text.contains(&format!("exec agentmux-bashwrap git-hook {name} \"$@\"")), "{name}");
        }
        // A second call finds them right and leaves them.
        let before = std::fs::metadata(dir.join("pre-push")).unwrap().modified().unwrap();
        ensure_hooks_dir(root.path()).unwrap();
        assert_eq!(std::fs::metadata(dir.join("pre-push")).unwrap().modified().unwrap(), before);
        // One that was changed is put back.
        std::fs::write(dir.join("pre-push"), "#!/bin/sh\nexit 0\n").unwrap();
        ensure_hooks_dir(root.path()).unwrap();
        assert!(std::fs::read_to_string(dir.join("pre-push")).unwrap().contains("git-hook pre-push"));
    }
}
