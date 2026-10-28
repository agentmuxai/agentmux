// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Makes the build-slot shims win in the shells this wrapper starts.
//!
//! Every Bash call of an agent runs under this process, and bash imports functions from its
//! environment, so exporting `cargo`, `task`, `npm`, `npx` and `docker` as functions that call the
//! shims (`agentmux_common::build_slot::shim_function_env`) puts the slot in front of those
//! tools however the shell's profile ordered the PATH. Without it, a `cargo.exe` in `~/bin`
//! (ahead of the shims' directory in the CLI's shell snapshot) is found first and the slot is
//! skipped: docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §4.

/// Sets the functions in this process's environment, which the bash it starts inherits.
pub(crate) fn export() {
    for (name, body) in agentmux_common::build_slot::shim_function_env_for_current_user() {
        std::env::set_var(name, body);
    }
}



#[cfg(test)]
mod tests {
    /// The `gh` function runs plain `gh` without the switch, and with the broker's token (or none)
    /// with it. A fake `agentmux-bashwrap` and `gh` on PATH stand in for the real ones.
    #[test]
    fn the_gh_function_uses_the_brokers_token_or_none() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(bash) = crate::bash_wrap::locate_bash() else { return };
        let dir = std::env::temp_dir().join(format!("amx-gh-fn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        script("gh", "echo \"gh GH_TOKEN=[$GH_TOKEN] GITHUB_TOKEN=[$GITHUB_TOKEN] args=$*\"");
        let run = |broker: &str, bashwrap_prints: &str, inherited: &str| {
            script("agentmux-bashwrap", &format!("[ \"$1\" = github-token ] && printf '%s' '{bashwrap_prints}'"));
            let path = std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap();
            let out = std::process::Command::new(&bash)
                .args(["-c", "gh pr list"])
                .env(agentmux_common::github_broker_shim::GH_FUNCTION_NAME, agentmux_common::github_broker_shim::GH_FUNCTION_BODY)
                .env("PATH", path)
                .env("AGENTMUX_GITHUB_BROKER", broker)
                .env("GH_TOKEN", inherited)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        // No switch: plain gh, with whatever the environment has.
        assert_eq!(run("", "ghs_X", "mine"), "gh GH_TOKEN=[mine] GITHUB_TOKEN=[] args=pr list");
        // The switch and a token: that token, for both names.
        assert_eq!(run("1", "ghs_X", "mine"), "gh GH_TOKEN=[ghs_X] GITHUB_TOKEN=[ghs_X] args=pr list");
        // The switch and no token: none, not the one the environment carried.
        assert_eq!(run("1", "", "mine"), "gh GH_TOKEN=[] GITHUB_TOKEN=[] args=pr list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `git` function asks `git-env` before a push or a commit: a refusal stops git, what it
    /// prints becomes the command's environment, and every other git command is plain `git`. The
    /// `gh` function gives `gh pr create` the arguments `github-owner-tag` prints.
    #[test]
    fn the_git_function_follows_git_env_and_the_gh_function_takes_the_owner_tag_arguments() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(bash) = crate::bash_wrap::locate_bash() else { return };
        let dir = std::env::temp_dir().join(format!("amx-git-fn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        script("git", "echo \"git AUTHOR=[$GIT_AUTHOR_NAME] OWNER=[$AGENTMUX_OWNER_GITHUB_ID] args=$*\"");
        script("gh", "echo \"gh args=$*\"");
        // What the real helper prints: the refusal, the environment lines, the arguments with a NUL after each.
        script(
            "agentmux-bashwrap",
            r#"case "$1" in
git-env)
  if [ -n "$FAKE_REFUSE" ]; then echo 'agentmux: refused' >&2; exit 1; fi
  if [ -n "$FAKE_ENV" ]; then printf 'GIT_AUTHOR_NAME=muxagent[bot]\nAGENTMUX_OWNER_GITHUB_ID=42\n'; fi;;
github-owner-tag)
  shift; shift
  for a in "$@"; do printf '%s\0' "$a"; done
  printf '%s\0' --tagged;;
github-token) printf 'ghs_X';;
esac"#,
        );
        let path = std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap();
        let run = |command: &str, broker: &str, refuse: &str, env: &str| {
            let out = std::process::Command::new(&bash)
                .args(["-c", command])
                .env(agentmux_common::github_broker_shim::GIT_FUNCTION_NAME, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY)
                .env(agentmux_common::github_broker_shim::GH_FUNCTION_NAME, agentmux_common::github_broker_shim::GH_FUNCTION_BODY)
                .env("PATH", &path)
                .env("AGENTMUX_GITHUB_BROKER", broker)
                .env("FAKE_REFUSE", refuse)
                .env("FAKE_ENV", env)
                // This test may itself run under an agent's git setup.
                .env_remove("GIT_AUTHOR_NAME")
                .env_remove("AGENTMUX_OWNER_GITHUB_ID")
                .output()
                .unwrap();
            // Shown when an assertion below fails.
            eprintln!("`{command}` (broker {broker:?}) exited {:?}, stderr: {}", out.status.code(), String::from_utf8_lossy(&out.stderr));
            (String::from_utf8_lossy(&out.stdout).trim().to_string(), out.status.code())
        };
        // A commit with the switch runs with what git-env printed.
        assert_eq!(run("git commit -m 'a b'", "1", "", "1").0, "git AUTHOR=[muxagent[bot]] OWNER=[42] args=commit -m a b");
        // git-env printing nothing leaves the commit as it is.
        assert_eq!(run("git commit -m x", "1", "", "").0, "git AUTHOR=[] OWNER=[] args=commit -m x");
        // A refusal stops git, and its status is the command's.
        assert_eq!(run("git push --no-verify", "1", "1", ""), (String::new(), Some(1)));
        // Other git commands, and every command without the switch, are plain git.
        assert_eq!(run("git status", "1", "1", "1").0, "git AUTHOR=[] OWNER=[] args=status");
        assert_eq!(run("git commit -m x", "", "1", "1").0, "git AUTHOR=[] OWNER=[] args=commit -m x");
        // `gh pr create` runs with the arguments the owner-tag helper gave; `gh pr list` with its own.
        assert_eq!(run("gh pr create --body x", "1", "", "").0, "gh args=pr create --body x --tagged");
        assert_eq!(run("gh pr list", "1", "", "").0, "gh args=pr list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A variable set with `std::env::set_var` under the name bash uses for exported functions
    /// reaches a bash started afterwards as a function (the mechanism `export` relies on).
    #[test]
    fn an_exported_function_variable_reaches_a_child_bash() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(bash) = crate::bash_wrap::locate_bash() else { return };
        std::env::set_var("BASH_FUNC_agentmux_probe%%", "() {  echo from-function \"$@\"\n}");
        let out = std::process::Command::new(bash).args(["-c", "agentmux_probe a b"]).output().unwrap();
        std::env::remove_var("BASH_FUNC_agentmux_probe%%");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-function a b");
    }
}
