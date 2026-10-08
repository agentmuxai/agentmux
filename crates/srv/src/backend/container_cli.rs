// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Installing the provider CLI inside a container agent's container.
//!
//! The public base image carries no provider CLI, so the first start installs
//! it from npm into a per-agent volume. This module is the pure part: what to
//! install, the marker that says it is done, and the two `sh` scripts. The
//! `docker exec` plumbing is in `ContainerManager::provision_cli`.
//!
//! Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md section 3.2.

use crate::backend::container_image::CONTAINER_CLI_DIR;
use crate::backend::providers;

/// What to install for one container agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInstall {
    pub provider_id: String,
    /// The command turns run, e.g. `claude`.
    pub command: String,
    /// `name@version` for the CLI and its companions.
    pub specs: Vec<String>,
    /// The pinned version, for the pane's "Installing Claude Code 2.1.288" row.
    pub version: String,
}

impl CliInstall {
    /// The install for `provider_id`, if the container runs that provider's own
    /// CLI by its usual name and the provider is installed from npm. `None`
    /// for a custom command or a provider that is not an npm package.
    pub fn for_provider(provider_id: &str, container_command: &str) -> Option<Self> {
        let provider = providers::get_provider(provider_id)?;
        if provider.npm_package.is_empty() || provider.pinned_version.is_empty() {
            return None;
        }
        if provider.cli_command != container_command {
            return None;
        }
        Some(Self {
            provider_id: provider.id.to_string(),
            command: provider.cli_command.to_string(),
            specs: provider.npm_install_specs(provider.pinned_version),
            version: provider.pinned_version.to_string(),
        })
    }

    /// File name inside [`CONTAINER_CLI_DIR`] that says this exact set of
    /// packages is installed. A new pin changes it, which is what makes the
    /// next start reinstall.
    pub fn marker(&self) -> String {
        let joined = self.specs.join("+");
        let safe: String = joined
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '+' { c } else { '-' })
            .collect();
        format!(".installed-{}", safe.trim_start_matches('-'))
    }

    /// Key for the "already checked" cache.
    pub fn cache_key(&self, container_name: &str) -> String {
        format!("{container_name}\u{0}{}", self.marker())
    }

    /// argv for the fast check; prints `present` or `missing`. The command on
    /// the image's own `PATH` counts as present: that is a legacy image with
    /// the CLI baked in, which must be left alone.
    pub fn check_argv(&self) -> Vec<String> {
        vec![
            "sh".into(),
            "-c".into(),
            CHECK_SCRIPT.into(),
            "agentmux-cli-check".into(),
            self.command.clone(),
            CONTAINER_CLI_DIR.into(),
            self.marker(),
        ]
    }

    /// argv for the install. Everything variable is a positional parameter,
    /// never spliced into the script text.
    pub fn install_argv(&self, limits: InstallLimits) -> Vec<String> {
        let mut argv = vec![
            "sh".into(),
            "-c".into(),
            INSTALL_SCRIPT.into(),
            "agentmux-cli-install".into(),
            CONTAINER_CLI_DIR.into(),
            self.marker(),
            self.command.clone(),
            limits.script.as_secs().max(1).to_string(),
        ];
        argv.extend(self.specs.iter().cloned());
        argv
    }
}

/// How long an install may run. The script limits npm itself (`script`), which
/// kills the whole process group; the host side (`run`) is the backstop for a
/// script that does not return, and is followed by [`reap_argv`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallLimits {
    pub run: std::time::Duration,
    pub script: std::time::Duration,
}

impl Default for InstallLimits {
    fn default() -> Self {
        Self {
            run: std::time::Duration::from_secs(600),
            script: std::time::Duration::from_secs(540),
        }
    }
}

/// Regex (in `pkill -f` / `pgrep -f` form) matching the install's `npm`. The
/// bracket keeps the reap script's own command line, which contains this text,
/// from matching itself.
const NPM_INSTALL_PATTERN: &str = "[n]pm install --prefix /home/agent/.agentmux/cli";

/// argv that kills any `npm install` into [`CONTAINER_CLI_DIR`] and waits for
/// it to be gone. Prints `gone` and exits 0 only once none is left.
pub fn reap_argv() -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        REAP_SCRIPT.into(),
        "agentmux-cli-reap".into(),
        NPM_INSTALL_PATTERN.into(),
    ]
}

/// `$1` the process pattern.
const REAP_SCRIPT: &str = r#"pkill -KILL -f "$1" || true
i=0
while pgrep -f "$1" >/dev/null 2>&1; do
    i=$((i+1))
    if [ "$i" -gt 20 ]; then echo alive; exit 1; fi
    sleep 0.5
done
echo gone"#;

/// Whether the reap script's output says nothing is left running.
pub fn reap_says_gone(output: &str) -> bool {
    output.lines().any(|l| l.trim() == "gone")
}

/// Why an install did not finish, in words for the pane.
///
/// `stopped` is whether the host-side backstop confirmed the install is no
/// longer running; `None` when the backstop did not apply.
pub fn install_failure_detail(exit: Option<i64>, timed_out: bool, stopped: Option<bool>, output: &str) -> String {
    if timed_out {
        return match stopped {
            Some(true) => "the install took too long and was stopped".to_string(),
            _ => "the install took too long and could not be stopped; wait a minute before trying again".to_string(),
        };
    }
    // `timeout` exits 124 when it had to kill npm.
    if exit == Some(124) || exit == Some(137) {
        return "the install took too long and was stopped".to_string();
    }
    let tail = output_tail(output, 600);
    if !tail.is_empty() {
        return tail;
    }
    format!("the install exited with status {}", exit.map_or("unknown".to_string(), |c| c.to_string()))
}

/// `$1` command, `$2` install dir, `$3` marker.
const CHECK_SCRIPT: &str = r#"if command -v "$1" >/dev/null 2>&1 || [ -f "$2/$3" ]; then echo present; else echo missing; fi"#;

/// `$1` install dir, `$2` marker, `$3` command, `$4` seconds npm may run, the
/// rest `name@version`.
///
/// npm installs into a staging directory inside the volume, so an attempt that
/// dies half-way never leaves a half-written `node_modules` where the CLI is
/// looked up. Only a verified install replaces the live tree, and the marker is
/// written last, so "marker present" always means "complete". The marker of an
/// older pin goes before the swap for the same reason.
const INSTALL_SCRIPT: &str = r#"set -eu
DIR="$1"; MARKER="$2"; BIN="$3"; LIMIT="$4"; shift 4
STAGE="$DIR/.staging"
mkdir -p "$DIR"
rm -rf "$STAGE"
mkdir -p "$STAGE"
export npm_config_cache=/tmp/agentmux-npm-cache npm_config_update_notifier=false
trap 'rm -rf /tmp/agentmux-npm-cache "$STAGE"' EXIT
timeout -k 15 "$LIMIT" npm install --prefix "$STAGE" --no-audit --no-fund --no-progress --loglevel=error "$@"
"$STAGE/node_modules/.bin/$BIN" --version >/dev/null
rm -f "$DIR"/.installed-*
rm -rf "$DIR/node_modules"
mv "$STAGE/node_modules" "$DIR/node_modules"
"$DIR/node_modules/.bin/$BIN" --version >/dev/null
: > "$DIR/$MARKER""#;

/// The directory the turn wrapper appends to `PATH`.
pub fn cli_bin_dir() -> String {
    format!("{CONTAINER_CLI_DIR}/node_modules/.bin")
}

/// Whether the check script's output means the CLI is already there.
pub fn check_says_present(output: &str) -> bool {
    output.lines().any(|l| l.trim() == "present")
}

/// The last `max` characters of `output`, whole lines where possible, trimmed.
pub fn output_tail(output: &str, max: usize) -> String {
    let trimmed = output.trim();
    let count = trimmed.chars().count();
    if count <= max {
        return trimmed.to_string();
    }
    let tail: String = trimmed.chars().skip(count - max).collect();
    match tail.find('\n') {
        Some(i) if i + 1 < tail.len() => tail[i + 1..].trim().to_string(),
        _ => tail.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> CliInstall {
        CliInstall::for_provider("claude", "claude").expect("claude installs from npm")
    }

    #[test]
    fn claude_installs_the_version_the_provider_table_pins() {
        let install = claude();
        let provider = providers::get_provider("claude").unwrap();
        assert_eq!(install.version, provider.pinned_version);
        assert_eq!(install.specs, vec![format!("@anthropic-ai/claude-code@{}", provider.pinned_version)]);
        assert_eq!(install.command, "claude");
    }

    #[test]
    fn a_custom_command_or_a_non_npm_provider_is_not_installed() {
        assert_eq!(CliInstall::for_provider("claude", "my-claude-wrapper"), None);
        assert_eq!(CliInstall::for_provider("no-such-provider", "x"), None);
    }

    #[test]
    fn the_marker_changes_with_the_pin_and_is_a_plain_file_name() {
        let a = claude();
        let mut b = a.clone();
        b.specs = vec!["@anthropic-ai/claude-code@9.9.9".into()];
        assert_ne!(a.marker(), b.marker());
        let marker = a.marker();
        assert!(marker.starts_with(".installed-"));
        assert!(!marker.contains('/') && !marker.contains('@') && !marker.contains(' '), "{marker}");
    }

    #[test]
    fn the_cache_key_separates_containers_and_pins() {
        let a = claude();
        assert_ne!(a.cache_key("agentmux-one"), a.cache_key("agentmux-two"));
    }

    #[test]
    fn the_scripts_get_their_inputs_as_positional_parameters() {
        let install = claude();
        let argv = install.install_argv(InstallLimits::default());
        assert_eq!(&argv[..2], ["sh", "-c"]);
        assert_eq!(argv[2], INSTALL_SCRIPT);
        assert_eq!(argv[4], CONTAINER_CLI_DIR);
        assert_eq!(argv[5], install.marker());
        assert_eq!(argv[6], "claude");
        assert_eq!(argv[7], "540", "npm's own limit is the script limit");
        assert_eq!(&argv[8..], install.specs.as_slice());
        // Nothing variable is part of the script text.
        assert!(!INSTALL_SCRIPT.contains("claude-code"));
        assert!(!INSTALL_SCRIPT.contains(&install.version));

        let check = install.check_argv();
        assert_eq!(check[2], CHECK_SCRIPT);
        assert_eq!(&check[4..], ["claude", CONTAINER_CLI_DIR, install.marker().as_str()]);
    }

    #[test]
    fn the_install_verifies_the_binary_before_writing_the_marker() {
        let verify = INSTALL_SCRIPT.rfind("--version").expect("verifies");
        let marker = INSTALL_SCRIPT.find(": > ").expect("writes the marker");
        assert!(verify < marker, "the marker is the last step, after the final check");
        assert!(INSTALL_SCRIPT.starts_with("set -eu"), "any failed step ends the script");
        assert!(INSTALL_SCRIPT.contains("rm -f \"$DIR\"/.installed-*"), "an older pin's marker goes");
    }

    #[test]
    fn the_install_builds_in_a_staging_dir_and_swaps_only_after_it_verified() {
        let npm = INSTALL_SCRIPT.find("npm install --prefix \"$STAGE\"").expect("npm installs into staging");
        let verify = INSTALL_SCRIPT.find("\"$STAGE/node_modules/.bin/$BIN\" --version").expect("verifies the staged binary");
        let drop_marker = INSTALL_SCRIPT.find("rm -f \"$DIR\"/.installed-*").expect("drops the old marker");
        let drop_live = INSTALL_SCRIPT.find("rm -rf \"$DIR/node_modules\"").expect("drops the live tree");
        let swap = INSTALL_SCRIPT.find("mv \"$STAGE/node_modules\"").expect("swaps");
        let write_marker = INSTALL_SCRIPT.find(": > ").expect("writes the marker");
        assert!(npm < verify && verify < drop_marker && drop_marker < drop_live && drop_live < swap && swap < write_marker);
        assert!(INSTALL_SCRIPT.contains("rm -rf \"$STAGE\"\n"), "a half-written earlier attempt is cleared first");
        assert!(INSTALL_SCRIPT.contains("timeout -k 15 \"$LIMIT\" npm install"), "npm limits itself");
    }

    #[test]
    fn the_reap_script_kills_the_install_and_waits_until_it_is_gone() {
        let argv = reap_argv();
        assert_eq!(argv[2], REAP_SCRIPT);
        assert!(REAP_SCRIPT.contains("pkill -KILL -f \"$1\""));
        assert!(REAP_SCRIPT.contains("pgrep -f \"$1\""), "it checks, not just signals");
        assert!(REAP_SCRIPT.contains("echo alive; exit 1"), "and fails when something survives");
        // The bracket keeps the script's own command line from matching itself.
        assert!(argv[4].starts_with("[n]pm install --prefix /home/agent/.agentmux/cli"));
        assert!(!argv[4].replace('[', "").replace(']', "").is_empty());
    }

    #[test]
    fn reap_output_is_read_by_line() {
        assert!(reap_says_gone("gone\n"));
        assert!(!reap_says_gone("alive\n"));
        assert!(!reap_says_gone(""));
    }

    #[test]
    fn a_timeout_reads_as_stopped_only_when_the_backstop_confirmed_it() {
        let stopped = install_failure_detail(None, true, Some(true), "");
        assert!(stopped.contains("was stopped"), "{stopped}");
        for unconfirmed in [Some(false), None] {
            let msg = install_failure_detail(None, true, unconfirmed, "");
            assert!(msg.contains("could not be stopped"), "{msg}");
            assert!(msg.contains("wait"), "tells the person not to retry at once: {msg}");
        }
    }

    #[test]
    fn the_scripts_own_timeout_and_a_kill_read_as_too_slow() {
        for code in [124, 137] {
            let msg = install_failure_detail(Some(code), false, None, "npm warn something");
            assert!(msg.contains("took too long"), "{msg}");
        }
    }

    #[test]
    fn an_ordinary_failure_shows_npms_own_last_words_or_the_status() {
        assert_eq!(install_failure_detail(Some(1), false, None, "npm error code ENOTFOUND\n"), "npm error code ENOTFOUND");
        assert_eq!(install_failure_detail(Some(1), false, None, ""), "the install exited with status 1");
        assert_eq!(install_failure_detail(None, false, None, ""), "the install exited with status unknown");
    }

    #[test]
    fn the_install_keeps_the_npm_cache_out_of_the_container_layer() {
        assert!(INSTALL_SCRIPT.contains("npm_config_cache=/tmp/"));
        assert!(INSTALL_SCRIPT.contains("rm -rf /tmp/agentmux-npm-cache"));
    }

    #[test]
    fn the_bin_dir_is_where_npm_puts_the_binary() {
        assert_eq!(cli_bin_dir(), "/home/agent/.agentmux/cli/node_modules/.bin");
    }

    #[test]
    fn the_check_output_is_read_by_line() {
        assert!(check_says_present("present\n"));
        assert!(check_says_present("noise\npresent\r\n"));
        assert!(!check_says_present("missing\n"));
        assert!(!check_says_present(""));
    }

    #[test]
    fn the_output_tail_keeps_the_end_on_a_line_boundary() {
        assert_eq!(output_tail("  short  ", 100), "short");
        let long = format!("{}\nlast line one\nlast line two", "x".repeat(500));
        let tail = output_tail(&long, 40);
        assert!(tail.ends_with("last line two"), "{tail}");
        assert!(!tail.contains('x'), "{tail}");
    }

    /// Runs the real scripts in a shell, with `npm` and the CLI faked.
    #[cfg(unix)]
    #[test]
    fn the_scripts_behave_in_a_real_shell() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;
        // The install script runs in the Linux agent container, which has
        // coreutils `timeout`; a host without it (macOS) can't run it.
        if !Command::new("sh").args(["-c", "command -v timeout"]).output().is_ok_and(|o| o.status.success()) {
            eprintln!("skipped: no `timeout` on this host; the script targets the Linux agent container");
            return;
        }
        let dir = std::env::temp_dir().join(format!("agentmux-cli-script-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("fakebin");
        std::fs::create_dir_all(&bin).unwrap();
        // A fake npm that "installs" a fake binary under --prefix.
        let npm = bin.join("npm");
        std::fs::write(
            &npm,
            "#!/bin/sh\nwhile [ \"$1\" != \"--prefix\" ]; do shift; done\nP=\"$2\"\nif [ -e \"$P/../.hang\" ]; then sleep 30; fi\nmkdir -p \"$P/node_modules/.bin\"\nprintf '#!/bin/sh\\necho 1.2.3\\n' > \"$P/node_modules/.bin/fakecli\"\nchmod +x \"$P/node_modules/.bin/fakecli\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cli_dir = dir.join("cli");
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let run = |script: &str, args: &[&str]| {
            let out = Command::new("sh")
                .arg("-c")
                .arg(script)
                .arg("t")
                .args(args)
                .env("PATH", &path)
                .output()
                .unwrap();
            (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
        };
        let dir_s = cli_dir.to_string_lossy().to_string();

        let (_, before) = run(CHECK_SCRIPT, &["fakecli", &dir_s, ".installed-x"]);
        assert!(!check_says_present(&before), "{before}");

        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-x", "fakecli", "60", "fake@1.2.3"]);
        assert!(ok, "install should succeed");
        assert!(cli_dir.join(".installed-x").is_file());

        let (_, after) = run(CHECK_SCRIPT, &["fakecli", &dir_s, ".installed-x"]);
        assert!(check_says_present(&after), "{after}");

        // A new pin leaves the old marker behind no longer.
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-y", "fakecli", "60", "fake@2.0.0"]);
        assert!(ok);
        assert!(!cli_dir.join(".installed-x").exists());
        assert!(cli_dir.join(".installed-y").is_file());

        // A binary that is not produced fails the script and writes no marker.
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-z", "missingcli", "60", "fake@3.0.0"]);
        assert!(!ok, "a missing binary must fail the install");
        assert!(!cli_dir.join(".installed-z").exists());
        assert!(!cli_dir.join(".staging").exists(), "staging is cleaned up on failure");

        // A half-written earlier attempt (stale staging, junk in the staging tree)
        // does not leak into a retry.
        std::fs::create_dir_all(cli_dir.join(".staging/node_modules/junk")).unwrap();
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-w", "fakecli", "60", "fake@4.0.0"]);
        assert!(ok);
        assert!(cli_dir.join(".installed-w").is_file());
        assert!(!cli_dir.join("node_modules/junk").exists(), "stale staging content is not installed");

        // A hanging npm (the fake sleeps while `.hang` exists) is killed by the
        // script's own limit: the script fails, no marker is written, and the
        // previous good install is left as it was.
        std::fs::write(cli_dir.join(".hang"), "").unwrap();
        let started = std::time::Instant::now();
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-v", "fakecli", "1", "fake@5.0.0"]);
        assert!(!ok, "the script gives up on a hanging npm");
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "it did not wait for the sleep");
        assert!(!cli_dir.join(".installed-v").exists());
        assert!(cli_dir.join(".installed-w").is_file(), "the previous install and its marker are intact");
        assert!(cli_dir.join("node_modules/.bin/fakecli").exists());
        assert!(!cli_dir.join(".staging").exists());
        std::fs::remove_file(cli_dir.join(".hang")).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
