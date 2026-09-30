// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shell integration script deployment and shell startup configuration.
//!
//! Embeds shell integration scripts (bash, zsh, pwsh, fish) and deploys them to
//! `~/.agentmux/shell/<type>/` on first use or when the version changes.
//! The shell controller uses these scripts to install prompt hooks that send
//! OSC 16162;E commands carrying `AGENTMUX_AGENT_ID`, enabling per-pane title
//! and color to work.

use std::path::{Path, PathBuf};

// ─── Embedded scripts ────────────────────────────────────────────────────────

const BASH_SCRIPT: &str = include_str!("shellintegration/bash.sh");
const ZSH_SCRIPT: &str = include_str!("shellintegration/zsh.sh");
const PWSH_SCRIPT: &str = include_str!("shellintegration/pwsh.ps1");
const FISH_SCRIPT: &str = include_str!("shellintegration/fish.fish");
/// Shared muxlog core (Node). Deployed once at `<shell>/muxlog.mjs`; every
/// shell's `muxlog` function delegates to it. One tested implementation does log
/// discovery + NDJSON rendering + filtering for all shells.
const MUXLOG_JS: &str = include_str!("shellintegration/muxlog.mjs");
/// Shared muxspect core (Node) — muxlog's live-state sibling. Deployed once
/// at `<shell>/muxspect.mjs`; every shell's `muxspect` function delegates to
/// it. See docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md.
const MUXSPECT_JS: &str = include_str!("shellintegration/muxspect.mjs");
/// muxopen — launch an agent into a pane from a terminal (the constructive
/// sibling of the stop verbs; REPORT_AGENT_OPEN_API_GAP_2026_09_06.md).
/// Deployed beside muxlog.mjs/muxspect.mjs; shell `muxopen` functions
/// delegate here.
const MUXOPEN_JS: &str = include_str!("shellintegration/muxopen.mjs");
/// muxsh — open editor/browser panes from a terminal
/// (REPORT_WSH_STYLE_CLI_FOR_AGENT_APP_API_2026_09_16.md,
/// SPEC_MUXSH_CLI_2026_09_16.md). Deployed beside its three siblings; shell
/// `muxsh` functions delegate here.
const MUXSH_JS: &str = include_str!("shellintegration/muxsh.mjs");
/// Shared auth-env-read + authenticated-fetch plumbing every core above
/// (except muxlog, which does no REST calls) imports via a relative
/// `./lib/muxclient.mjs` — so it must be deployed at `<shell>/lib/`, not
/// flat beside its callers. See
/// docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §2.8.
const MUXCLIENT_JS: &str = include_str!("shellintegration/lib/muxclient.mjs");

/// Deployment marker: `<package version>-<content hash>`, NOT the bare
/// package version alone (codex P2 on PR #2380). Local/dev builds routinely
/// iterate on these embedded scripts WITHOUT a version bump (this repo's own
/// convention — see CLAUDE.md's "Build versioning — local builds are
/// *labeled*, not *versioned*"), so a version-only marker left `.version`
/// already matching on every same-version rebuild/restart, silently
/// skipping deployment of a newly-added or newly-edited script (muxspect.mjs
/// specifically, added in the same PR that added the startup call site —
/// the marker never invalidated to pick it up). Hashing the actual embedded
/// content means ANY change to ANY script forces a redeploy regardless of
/// whether the package version moved, while an unchanged rebuild still
/// skips the (cheap, but not free) disk writes.
fn version_marker() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    BASH_SCRIPT.hash(&mut hasher);
    ZSH_SCRIPT.hash(&mut hasher);
    PWSH_SCRIPT.hash(&mut hasher);
    FISH_SCRIPT.hash(&mut hasher);
    MUXLOG_JS.hash(&mut hasher);
    MUXSPECT_JS.hash(&mut hasher);
    // MUXOPEN_JS was missing here (an edit to muxopen.mjs alone never changed
    // this marker, so a running instance never redeployed it) — fixed while
    // adding MUXSH_JS below, same class of bug this function's own doc
    // comment already warns about.
    MUXOPEN_JS.hash(&mut hasher);
    MUXSH_JS.hash(&mut hasher);
    MUXCLIENT_JS.hash(&mut hasher);
    format!("{}-{:x}", env!("CARGO_PKG_VERSION"), hasher.finish())
}

// ─── Shell type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShellType {
    Bash,
    Zsh,
    Pwsh,
    Fish,
    Unknown,
}

/// Detect shell type from the shell binary path.
pub fn detect_shell_type(shell_path: &str) -> ShellType {
    let name = Path::new(shell_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    match name.as_str() {
        "pwsh" | "powershell" => ShellType::Pwsh,
        "bash" => ShellType::Bash,
        "zsh" => ShellType::Zsh,
        "fish" => ShellType::Fish,
        _ => ShellType::Unknown,
    }
}

// ─── Deploy ──────────────────────────────────────────────────────────────────

/// Deploy shell integration scripts into `<shell_root>/<type>/`.
/// Skips deployment if the version marker is already current.
/// Errors are logged but not fatal — a missing script just means no integration.
/// Root under which this instance's shell-integration scripts are deployed.
///
/// Deliberately under the user's home rather than the instance data dir:
/// MSIX virtualises writes to `%LocalAppData%`, so scripts written there are
/// invisible to the ConPTY-spawned children that must source them, while `~`
/// is never virtualised (see `shell/lifecycle.rs`'s spawn path).
///
/// Keyed per instance, because the home dir is otherwise machine-global and
/// every instance of every version writes these same files — on every shell
/// spawn, not just at boot. Two concurrently-running versions therefore
/// rewrite each other's scripts indefinitely, and a shell can source an
/// rcfile another instance is mid-replacement of. That is invariant **I6**
/// (instances of different `(channel, version)` never share a directory),
/// and the key mirrors **I5**'s `dir_hash` treatment of named OS objects.
///
/// The data dir is the right key: it is `<channel>/versions/<version>/data`
/// by construction, so it already distinguishes exactly the pairs I6 names.
pub fn integration_base_for(data_dir: &Path, home_dir: &Path) -> PathBuf {
    let canonical = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
    let key = format!("{:016x}", agentmux_common::runtime_mode::fnv1a_64(
        canonical.to_string_lossy().to_lowercase().as_bytes(),
    ));
    home_dir.join(".agentmux").join("shell").join(key)
}

/// The stable, user-facing script root: `~/.agentmux/shell`.
///
/// `docs/MUXSPECT.md` and `docs/MUXSH.md` document invoking these directly
/// (`node ~/.agentmux/shell/muxspect.mjs …`) as the fallback when the shell
/// function isn't defined. That path has to keep working and has to stay
/// predictable — a user cannot know their instance's hash — so it is still
/// deployed, shared, last-writer-wins. Nothing sources it at shell startup;
/// the rcfiles a terminal actually loads come from `integration_base`.
pub fn documented_shell_root() -> PathBuf {
    crate::backend::base::get_mux_data_dir().join("shell")
}

/// `integration_base_for` bound to this process's real data dir and home.
///
/// Ordering note: `get_mux_data_dir()` reads `AGENTMUX_DATA_HOME`, which
/// `bootstrap::open_stores_and_migrate` overwrites from the resolved
/// `config.data_home` (CLI `--wavedata` winning over the launcher's env). Until
/// that runs, the variable may still hold a value INHERITED from whichever
/// instance spawned this process — observed live: a dev srv started from a
/// terminal inside a 0.56.2 instance carried that instance's data dir. Both
/// call sites (bootstrap's own deploy, and the shell spawn in
/// `blockcontroller/shell/lifecycle.rs`) run after it, so both see the
/// corrected value. Do not call this earlier in startup without checking that.
pub fn integration_base() -> PathBuf {
    integration_base_for(
        &crate::backend::base::get_mux_data_dir(),
        &crate::backend::base::get_home_dir(),
    )
}

pub fn deploy_scripts(shell_root: &Path) {
    let shell_base = shell_root.to_path_buf();
    let version_file = shell_base.join(".version");
    let marker = version_marker();

    // Check if already up-to-date
    if let Ok(existing) = std::fs::read_to_string(&version_file) {
        if existing.trim() == marker {
            return;
        }
    }

    tracing::info!("Deploying shell integration scripts ({})", marker);

    let deploys: &[(&str, &str, &str)] = &[
        ("bash", ".bashrc", BASH_SCRIPT),
        ("zsh", ".zshrc", ZSH_SCRIPT),
        ("pwsh", "wavepwsh.ps1", PWSH_SCRIPT),
        ("fish", "wave.fish", FISH_SCRIPT),
    ];

    let mut all_ok = true;
    for (dir_name, file_name, content) in deploys {
        let dir = shell_base.join(dir_name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("shell integration: failed to create {}: {}", dir.display(), e);
            all_ok = false;
            continue;
        }
        let path = dir.join(file_name);
        if let Err(e) = std::fs::write(&path, content) {
            tracing::warn!("shell integration: failed to write {}: {}", path.display(), e);
            all_ok = false;
        }
    }

    // Deploy the shared muxlog core next to the per-shell dirs. Each rcfile
    // resolves it relative to its own location and delegates to `node muxlog.mjs`.
    let muxlog_path = shell_base.join("muxlog.mjs");
    if let Err(e) = std::fs::write(&muxlog_path, MUXLOG_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxlog_path.display(), e);
        all_ok = false;
    }

    // Deploy the shared muxspect core the same way, right next to muxlog.mjs.
    let muxspect_path = shell_base.join("muxspect.mjs");
    if let Err(e) = std::fs::write(&muxspect_path, MUXSPECT_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxspect_path.display(), e);
    }
    // muxopen deploys the same way, next to its two siblings.
    let muxopen_path = shell_base.join("muxopen.mjs");
    if let Err(e) = std::fs::write(&muxopen_path, MUXOPEN_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxopen_path.display(), e);
        all_ok = false;
    }
    // muxsh deploys the same way, next to its three siblings.
    let muxsh_path = shell_base.join("muxsh.mjs");
    if let Err(e) = std::fs::write(&muxsh_path, MUXSH_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxsh_path.display(), e);
        all_ok = false;
    }
    // muxclient.mjs is imported via a relative `./lib/muxclient.mjs` from
    // muxspect/muxopen/muxsh, so it must land in its own lib/ subdirectory,
    // not flat beside them.
    let lib_dir = shell_base.join("lib");
    if let Err(e) = std::fs::create_dir_all(&lib_dir) {
        tracing::warn!("shell integration: failed to create {}: {}", lib_dir.display(), e);
        all_ok = false;
    } else {
        let muxclient_path = lib_dir.join("muxclient.mjs");
        if let Err(e) = std::fs::write(&muxclient_path, MUXCLIENT_JS) {
            tracing::warn!("shell integration: failed to write {}: {}", muxclient_path.display(), e);
            all_ok = false;
        }
    }

    // Write version marker only if all scripts deployed successfully
    if all_ok {
        let _ = std::fs::write(&version_file, &marker);
    }
}

// ─── Startup configuration ───────────────────────────────────────────────────

/// Shell startup configuration: extra args and env vars to inject.
pub struct ShellStartup {
    /// Extra args to append to the shell command.
    pub extra_args: Vec<String>,
    /// Environment variables to set in the PTY.
    pub env_vars: Vec<(String, String)>,
}

/// Get the startup configuration for launching an interactive shell with
/// AgentMux integration. Returns `None` for unknown shell types.
pub fn get_shell_startup(
    shell_type: ShellType,
    shell_root: &Path,
) -> Option<ShellStartup> {
    match shell_type {
        ShellType::Bash => {
            let rcfile = shell_root.join("bash").join(".bashrc");
            Some(ShellStartup {
                extra_args: vec![
                    "--rcfile".to_string(),
                    rcfile.to_string_lossy().into_owned(),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Zsh => {
            let zdotdir = shell_root.join("zsh");
            Some(ShellStartup {
                extra_args: vec![],
                env_vars: vec![
                    ("ZDOTDIR".to_string(), zdotdir.to_string_lossy().into_owned()),
                    // Preserve original ZDOTDIR so the integration script can source ~/.zshrc
                    ("AGENTMUX_ZDOTDIR".to_string(), zdotdir.to_string_lossy().into_owned()),
                ],
            })
        }
        ShellType::Pwsh => {
            let script = shell_root
                .join("pwsh")
                .join("wavepwsh.ps1");
            Some(ShellStartup {
                extra_args: vec![
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-NoExit".to_string(),
                    "-File".to_string(),
                    script.to_string_lossy().into_owned(),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Fish => {
            let script = shell_root
                .join("fish")
                .join("wave.fish");
            Some(ShellStartup {
                extra_args: vec![
                    "-C".to_string(),
                    format!("source {}", shell_quote(&script.to_string_lossy())),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Unknown => None,
    }
}

// wsh has been retired — see docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md.
// The `AGENTMUX` env var is now a plain "1" sentinel, not a path.

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Single-quote a path for POSIX shell usage.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell_type() {
        assert_eq!(detect_shell_type("bash"), ShellType::Bash);
        assert_eq!(detect_shell_type("/bin/bash"), ShellType::Bash);
        assert_eq!(detect_shell_type("zsh"), ShellType::Zsh);
        assert_eq!(detect_shell_type("/usr/bin/zsh"), ShellType::Zsh);
        assert_eq!(detect_shell_type("pwsh"), ShellType::Pwsh);
        assert_eq!(detect_shell_type("powershell"), ShellType::Pwsh);
        assert_eq!(detect_shell_type("fish"), ShellType::Fish);
        assert_eq!(detect_shell_type("cmd.exe"), ShellType::Unknown);
        assert_eq!(detect_shell_type("cmd"), ShellType::Unknown);
    }

    #[test]
    fn test_bash_startup_args() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Bash, dir).unwrap();
        assert_eq!(startup.extra_args[0], "--rcfile");
        assert!(startup.extra_args[1].contains("bash"));
        assert!(startup.extra_args[1].ends_with(".bashrc"));
    }

    #[test]
    fn test_pwsh_startup_args() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Pwsh, dir).unwrap();
        assert!(startup.extra_args.contains(&"-NoExit".to_string()));
        assert!(startup.extra_args.contains(&"-File".to_string()));
    }

    #[test]
    fn test_zsh_uses_zdotdir() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Zsh, dir).unwrap();
        assert!(startup.extra_args.is_empty());
        assert!(startup.env_vars.iter().any(|(k, _)| k == "ZDOTDIR"));
    }

    #[test]
    fn test_unknown_shell_returns_none() {
        let dir = Path::new("/tmp");
        assert!(get_shell_startup(ShellType::Unknown, dir).is_none());
    }

    /// `muxspect.mjs` deploys next to `muxlog.mjs` — codified as a test since
    /// it's easy to add a new embedded script and forget the deploy line
    /// (see docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md).
    #[test]
    fn deploy_scripts_writes_muxlog_and_muxspect_side_by_side() {
        let tmp = std::env::temp_dir().join(format!(
            "agentmux-shellintegration-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        deploy_scripts(&tmp.join("shell"));

        let shell_base = tmp.join("shell");
        let muxlog = shell_base.join("muxlog.mjs");
        let muxspect = shell_base.join("muxspect.mjs");
        let muxopen = shell_base.join("muxopen.mjs");
        let muxsh = shell_base.join("muxsh.mjs");
        let muxclient = shell_base.join("lib").join("muxclient.mjs");
        assert!(muxlog.exists(), "muxlog.mjs should be deployed");
        assert!(muxspect.exists(), "muxspect.mjs should be deployed alongside it");
        assert!(muxopen.exists(), "muxopen.mjs should be deployed alongside them");
        assert!(muxsh.exists(), "muxsh.mjs should be deployed alongside them");
        assert!(
            muxclient.exists(),
            "lib/muxclient.mjs should be deployed — muxspect/muxopen/muxsh import it via a relative path"
        );
        assert_eq!(std::fs::read_to_string(&muxspect).unwrap(), MUXSPECT_JS);
        assert_eq!(std::fs::read_to_string(&muxsh).unwrap(), MUXSH_JS);
        assert_eq!(std::fs::read_to_string(&muxclient).unwrap(), MUXCLIENT_JS);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// codex P2 on PR #2380: a version-only marker left an EXISTING profile
    /// (same CARGO_PKG_VERSION, older content) permanently skipping
    /// deployment of a newly-added script — exactly what an existing
    /// `~/.agentmux` from a prior same-version dev build would have. Content
    /// hashing must force a redeploy in that case, not just on a genuinely
    /// fresh profile (the only case the test above covers).
    #[test]
    fn deploy_scripts_redeploys_when_marker_predates_a_content_change() {
        let tmp = std::env::temp_dir().join(format!(
            "agentmux-shellintegration-test-stale-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let shell_base = tmp.join("shell");
        std::fs::create_dir_all(&shell_base).unwrap();

        // Simulate a profile deployed by an older build of shellintegration.rs
        // that only ever wrote the bare package version as its marker (i.e.
        // BEFORE this fix existed) — the exact "same version, no muxspect.mjs
        // yet" scenario codex's finding describes.
        std::fs::write(shell_base.join(".version"), env!("CARGO_PKG_VERSION")).unwrap();
        assert!(!shell_base.join("muxspect.mjs").exists(), "precondition: not deployed yet");

        deploy_scripts(&tmp.join("shell"));

        assert!(
            shell_base.join("muxspect.mjs").exists(),
            "a stale version-only marker must not suppress deployment of a script that was never written"
        );
        assert_eq!(
            std::fs::read_to_string(shell_base.join(".version")).unwrap(),
            version_marker(),
            "marker must be updated to the new content-hashed form"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Per-instance isolation (I6) ──────────────────────────────────────

    /// THE invariant. Two instances differing only by channel or version must
    /// never deploy into the same directory — otherwise each shell spawn in
    /// one rewrites the scripts the other's shells source.
    #[test]
    fn two_instances_never_share_a_shell_integration_dir() {
        let home = Path::new("/home/u");
        let a = integration_base_for(Path::new("/ch/alpha/versions/0.56.2/data"), home);
        let b = integration_base_for(Path::new("/ch/alpha/versions/0.56.3/data"), home);
        let c = integration_base_for(Path::new("/ch/beta/versions/0.56.3/data"), home);
        assert_ne!(a, b, "same channel, different version must not collide");
        assert_ne!(b, c, "same version, different channel must not collide");
        assert_ne!(a, c);
    }

    /// The path is recomputed on every shell spawn, so an unstable key would
    /// send a later shell to a directory nothing was deployed into.
    #[test]
    fn the_same_instance_always_resolves_to_the_same_dir() {
        let home = Path::new("/home/u");
        let d = Path::new("/ch/alpha/versions/0.56.3/data");
        assert_eq!(integration_base_for(d, home), integration_base_for(d, home));
    }

    /// Must stay under the home dir: MSIX virtualises `%LocalAppData%`, so a
    /// data-dir-relative path would be invisible to the shells that source it.
    #[test]
    fn deployment_stays_under_the_never_virtualised_home_dir() {
        let home = Path::new("/home/u");
        let base = integration_base_for(Path::new("/ch/alpha/versions/0.56.3/data"), home);
        assert!(base.starts_with(home), "must live under home, got {}", base.display());
        assert!(!base.starts_with("/ch/alpha"), "must not live under the data dir");
    }

    /// The coupling that actually matters: whatever `deploy_scripts` writes,
    /// `get_shell_startup` must point the shell AT. If these two drift apart,
    /// scripts land where nothing sources them and every terminal silently
    /// loses shell integration — with no error anywhere. Asserted by resolving
    /// the startup arg to a real file on disk, not by comparing two strings
    /// built the same way (which would pass even if both were wrong).
    #[test]
    fn the_deployed_rcfile_is_exactly_the_one_the_shell_is_told_to_load() {
        let tmp = std::env::temp_dir().join(format!("am-si-couple-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("shell");
        deploy_scripts(&root);

        let startup = get_shell_startup(ShellType::Bash, &root).expect("bash startup");
        assert_eq!(startup.extra_args[0], "--rcfile");
        let rcfile = Path::new(&startup.extra_args[1]);
        assert!(
            rcfile.exists(),
            "shell is told to load {} but deploy_scripts never wrote it",
            rcfile.display()
        );

        // And it must be the real script, not an empty placeholder.
        let body = std::fs::read_to_string(rcfile).expect("read rcfile");
        assert!(body.contains("_agentmux_si_prompt_command"), "rcfile is not the integration script");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
