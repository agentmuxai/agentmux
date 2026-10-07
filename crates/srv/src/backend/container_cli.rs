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
    pub fn install_argv(&self) -> Vec<String> {
        let mut argv = vec![
            "sh".into(),
            "-c".into(),
            INSTALL_SCRIPT.into(),
            "agentmux-cli-install".into(),
            CONTAINER_CLI_DIR.into(),
            self.marker(),
            self.command.clone(),
        ];
        argv.extend(self.specs.iter().cloned());
        argv
    }
}

/// `$1` command, `$2` install dir, `$3` marker.
const CHECK_SCRIPT: &str = r#"if command -v "$1" >/dev/null 2>&1 || [ -f "$2/$3" ]; then echo present; else echo missing; fi"#;

/// `$1` install dir, `$2` marker, `$3` command, the rest `name@version`.
const INSTALL_SCRIPT: &str = r#"set -eu
DIR="$1"; MARKER="$2"; BIN="$3"; shift 3
mkdir -p "$DIR"
export npm_config_cache=/tmp/agentmux-npm-cache npm_config_update_notifier=false
trap 'rm -rf /tmp/agentmux-npm-cache' EXIT
npm install --prefix "$DIR" --no-audit --no-fund --no-progress --loglevel=error "$@"
"$DIR/node_modules/.bin/$BIN" --version >/dev/null
rm -f "$DIR"/.installed-*
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
        let argv = install.install_argv();
        assert_eq!(&argv[..2], ["sh", "-c"]);
        assert_eq!(argv[2], INSTALL_SCRIPT);
        assert_eq!(argv[4], CONTAINER_CLI_DIR);
        assert_eq!(argv[5], install.marker());
        assert_eq!(argv[6], "claude");
        assert_eq!(&argv[7..], install.specs.as_slice());
        // Nothing variable is part of the script text.
        assert!(!INSTALL_SCRIPT.contains("claude-code"));
        assert!(!INSTALL_SCRIPT.contains(&install.version));

        let check = install.check_argv();
        assert_eq!(check[2], CHECK_SCRIPT);
        assert_eq!(&check[4..], ["claude", CONTAINER_CLI_DIR, install.marker().as_str()]);
    }

    #[test]
    fn the_install_verifies_the_binary_before_writing_the_marker() {
        let verify = INSTALL_SCRIPT.find("--version").expect("verifies");
        let marker = INSTALL_SCRIPT.find(": > ").expect("writes the marker");
        assert!(verify < marker);
        assert!(INSTALL_SCRIPT.starts_with("set -eu"), "any failed step ends the script");
        assert!(INSTALL_SCRIPT.contains("rm -f \"$DIR\"/.installed-*"), "an older pin's marker goes");
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
        let dir = std::env::temp_dir().join(format!("agentmux-cli-script-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("fakebin");
        std::fs::create_dir_all(&bin).unwrap();
        // A fake npm that "installs" a fake binary under --prefix.
        let npm = bin.join("npm");
        std::fs::write(
            &npm,
            "#!/bin/sh\nwhile [ \"$1\" != \"--prefix\" ]; do shift; done\nP=\"$2\"\nmkdir -p \"$P/node_modules/.bin\"\nprintf '#!/bin/sh\\necho 1.2.3\\n' > \"$P/node_modules/.bin/fakecli\"\nchmod +x \"$P/node_modules/.bin/fakecli\"\n",
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

        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-x", "fakecli", "fake@1.2.3"]);
        assert!(ok, "install should succeed");
        assert!(cli_dir.join(".installed-x").is_file());

        let (_, after) = run(CHECK_SCRIPT, &["fakecli", &dir_s, ".installed-x"]);
        assert!(check_says_present(&after), "{after}");

        // A new pin leaves the old marker behind no longer.
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-y", "fakecli", "fake@2.0.0"]);
        assert!(ok);
        assert!(!cli_dir.join(".installed-x").exists());
        assert!(cli_dir.join(".installed-y").is_file());

        // A binary that is not produced fails the script and writes no marker.
        let (ok, _) = run(INSTALL_SCRIPT, &[&dir_s, ".installed-z", "missingcli", "fake@3.0.0"]);
        assert!(!ok, "a missing binary must fail the install");
        assert!(!cli_dir.join(".installed-z").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
