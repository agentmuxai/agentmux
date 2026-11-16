// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `gh` as a program, for the callers the `gh` shell function never reaches.
//!
//! An agent's `gh` is a bash function (`agentmux_common::github_broker_shim::GH_FUNCTION_BODY`) that asks this
//! wrapper for the token of the repository the command is about. A function exists only in the bash that imported
//! it: `timeout 60 gh pr create`, `xargs gh`, `env gh`, `make`, a script with a `#!/bin/sh` line or a Python
//! `subprocess` all start the real `gh` binary, which has no login in a container and says "gh auth login".
//!
//! In a container's image this wrapper is also installed as `/usr/local/bin/gh` (a symlink, ahead of the real `gh`
//! on PATH). Started under that name it does what the function does, with the function's own text, and then runs
//! the real `gh`. No token is stored: each call asks for one, as the function does.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use agentmux_common::github_broker_shim::GH_FUNCTION_BODY;

/// Where the function body finds the real `gh` (it says `command gh` in a shell, which would find this program).
const REAL_GH_ENV: &str = "AGENTMUX_REAL_GH";
/// Set by the function when it runs `gh`, so this program does not do the same work a second time.
const DONE_ENV: &str = "AGENTMUX_GH_SHIM_DONE";
/// Set by srv for an agent whose GitHub access goes through AgentMux.
const BROKER_ENV: &str = "AGENTMUX_GITHUB_BROKER";

/// Whether the program was started as `gh` (or `gh.exe`).
pub(crate) fn invoked_as_gh(argv0: &OsStr) -> bool {
    Path::new(argv0).file_stem().is_some_and(|s| s.eq_ignore_ascii_case("gh"))
}

/// The shell text that is the function, calling the real `gh` by the path in [`REAL_GH_ENV`], then running it.
fn script() -> String {
    format!("gh {}\ngh \"$@\"", GH_FUNCTION_BODY.replace("command gh ", &format!("\"${REAL_GH_ENV}\" ")))
}

/// The first `gh` on `path` that is not `me` (this program, which the symlink makes one of them).
pub(crate) fn real_gh_in(path: &OsStr, me: Option<&Path>) -> Option<PathBuf> {
    let me = me.map(|m| m.canonicalize().unwrap_or_else(|_| m.to_path_buf()));
    std::env::split_paths(path).map(|d| d.join("gh")).find(|c| {
        c.is_file() && me.as_ref().is_none_or(|me| &c.canonicalize().unwrap_or_else(|_| c.clone()) != me)
    })
}

/// The command that runs `gh` with `args`: through the function's steps when the agent's GitHub access is
/// brokered and the function has not run them already, else the real `gh` as it is.
fn command_for(bash: &Path, real: &Path, args: &[OsString], brokered: bool) -> Command {
    let mut cmd = if brokered {
        let mut cmd = Command::new(bash);
        cmd.arg("-c").arg(script()).arg("gh").args(args).env(REAL_GH_ENV, real);
        cmd
    } else {
        let mut cmd = Command::new(real);
        cmd.args(args);
        cmd
    };
    // Only a container's image has this program as `gh`; the flag keeps a console from opening if it is ever run on Windows.
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    cmd
}

fn launch(mut cmd: Command) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // exec only returns when it failed.
        let err = cmd.exec();
        eprintln!("agentmux: gh could not be started: {err}");
        127
    }
    #[cfg(not(unix))]
    {
        match cmd.status() {
            Ok(s) => s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("agentmux: gh could not be started: {e}");
                127
            }
        }
    }
}

/// When this program was started as `gh`: run `gh` and give the exit code. `None` for any other name.
pub(crate) fn run_if_invoked_as_gh() -> Option<i32> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    if !invoked_as_gh(argv.first()?) {
        return None;
    }
    let me = std::env::current_exe().ok();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(real) = std::env::var_os(REAL_GH_ENV).map(PathBuf::from).or_else(|| real_gh_in(&path, me.as_deref())) else {
        eprintln!("agentmux: the GitHub CLI (gh) is not installed here, so there is nothing to run. Ask your leader to update this agent's image.");
        return Some(127);
    };
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    let brokered = set(BROKER_ENV) && !set(DONE_ENV);
    Some(launch(command_for(Path::new("bash"), &real, &argv[1..], brokered)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(a: &[&str]) -> Vec<OsString> {
        a.iter().map(OsString::from).collect()
    }

    #[test]
    fn it_answers_to_the_name_gh_and_no_other() {
        assert!(invoked_as_gh(OsStr::new("/usr/local/bin/gh")));
        assert!(invoked_as_gh(OsStr::new("gh")));
        assert!(invoked_as_gh(OsStr::new("/usr/local/bin/GH.exe")));
        assert!(!invoked_as_gh(OsStr::new("/usr/local/bin/agentmux-bashwrap")));
        assert!(!invoked_as_gh(OsStr::new("ghost")));
    }

    /// Backslashes separate directories on Windows only.
    #[cfg(windows)]
    #[test]
    fn a_windows_path_names_gh_too() {
        assert!(invoked_as_gh(OsStr::new("C:\\bin\\GH.exe")));
    }

    #[test]
    fn the_script_is_the_function_calling_the_real_gh_by_path() {
        let s = script();
        assert!(!s.contains("command gh"), "{s}");
        assert!(s.contains("\"$AGENTMUX_REAL_GH\" \"$@\""), "{s}");
        assert!(s.starts_with("gh () {") || s.starts_with("gh() {") || s.starts_with("gh () "), "{s}");
        assert!(s.ends_with("gh \"$@\""), "{s}");
        // The same steps as the function: the container verbs, the text guard, the token, the owner tag.
        for step in ["github-container", "github-text-guard", "github-token", "github-owner-tag"] {
            assert!(s.contains(step), "{step}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_real_gh_is_the_first_one_that_is_not_this_program() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("amx-gh-shim-find-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let me = a.join("agentmux-bashwrap");
        std::fs::write(&me, "x").unwrap();
        symlink(&me, a.join("gh")).unwrap();
        std::fs::write(b.join("gh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(b.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(real_gh_in(&path, Some(&me)), Some(b.join("gh")));
        // Nothing else on PATH: no gh, so no answer, never this program again.
        assert_eq!(real_gh_in(&std::env::join_paths([&a]).unwrap(), Some(&me)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fake `agentmux-bashwrap` (the text guard passes the arguments on, the token helper prints `token`) and a
    /// fake real `gh` that says what it was given.
    fn run(brokered: bool, token: &str, inherited: &str) -> Option<String> {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bash = crate::bash_wrap::locate_bash().ok()?;
        let dir = std::env::temp_dir().join(format!("amx-gh-shim-run-{}-{}", std::process::id(), brokered as u8));
        std::fs::create_dir_all(&dir).unwrap();
        let script_file = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        };
        let real = script_file("real-gh", "echo \"gh GH_TOKEN=[$GH_TOKEN] GITHUB_TOKEN=[$GITHUB_TOKEN] DONE=[$AGENTMUX_GH_SHIM_DONE] args=$*\"");
        script_file(
            "agentmux-bashwrap",
            &format!("case \"$1\" in github-text-guard) shift; shift; for a in \"$@\"; do printf '%s\\0' \"$a\"; done; printf 'ok\\0';; github-token) printf '%s' '{token}';; esac"),
        );
        let path = std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap();
        let mut cmd = command_for(&bash, &real, &os(&["pr", "list"]), brokered);
        cmd.env("PATH", path).env(BROKER_ENV, "1").env("GH_TOKEN", inherited).env_remove("GITHUB_TOKEN").env_remove("AGENTMUX_IN_CONTAINER");
        #[cfg(windows)]
        {
            use agentmux_common::win32::NoWindow;
            cmd.no_window();
        }
        let out = cmd.output().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    #[test]
    fn a_brokered_call_runs_the_real_gh_with_the_repositorys_token_or_none() {
        // The token srv gave, for both names, and the marker that says the steps were done.
        let Some(got) = run(true, "ghs_X", "mine") else { return };
        assert_eq!(got, "gh GH_TOKEN=[ghs_X] GITHUB_TOKEN=[ghs_X] DONE=[1] args=pr list");
        // No token (a refusal): none, never the one the environment happened to carry.
        assert_eq!(run(true, "", "mine").unwrap(), "gh GH_TOKEN=[] GITHUB_TOKEN=[] DONE=[1] args=pr list");
    }

    /// Windows can't start a shell script as a program, so the fake real `gh` runs there only through bash (the test above).
    #[cfg(unix)]
    #[test]
    fn a_call_with_no_broker_is_the_real_gh_as_it_is() {
        let Some(got) = run(false, "ghs_X", "mine") else { return };
        assert_eq!(got, "gh GH_TOKEN=[mine] GITHUB_TOKEN=[] DONE=[] args=pr list");
    }
}
