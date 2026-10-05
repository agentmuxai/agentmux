// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Installing AgentMux's helper (`agentmux-remote`) on an SSH host, for
//! durable sessions (spec §6.2 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! 1. Probe the host over the pane's own ssh: `uname -sm`, and whether this
//!    version's helper is already at `~/.agentmux-remote/bin/<version>/`.
//! 2. Get the matching static build from this package, which carries all four
//!    (`tools/remote/<target>/`, spec §9.1; on macOS the Linux builds sit in
//!    `Contents/Resources/remote/<target>/`). A developer can point
//!    `AGENTMUX_REMOTE_HELPER_DIR` at their own builds
//!    (`scripts/build-remote-helpers.sh` writes the same layout).
//! 3. Upload it over that ssh (`cat >` a temp file in an owner-only folder;
//!    no scp or sftp needed on the host), check its hash there, then
//!    `chmod 700` and move it into place.
//!
//! Every remote script runs under `sh -c '...'`, so it works whatever the
//! user's login shell is.

use std::path::{Path, PathBuf};

/// The helper's build target for `uname -sm` output (`Linux x86_64`,
/// `Darwin arm64`, ...). `None` for a platform without a build.
pub fn target_for(uname: &str) -> Option<&'static str> {
    let mut words = uname.split_whitespace();
    let os = words.next()?;
    let arch = words.next()?;
    Some(match (os, arch) {
        ("Linux", "x86_64" | "amd64") => "x86_64-unknown-linux-musl",
        ("Linux", "aarch64" | "arm64") => "aarch64-unknown-linux-musl",
        ("Darwin", "x86_64") => "x86_64-apple-darwin",
        ("Darwin", "arm64" | "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    })
}

/// The flat name a build had as a release asset, `agentmux-remote-<version>-<target>`.
/// Still accepted in `AGENTMUX_REMOTE_HELPER_DIR` beside the packaged layout.
pub fn asset_name(version: &str, target: &str) -> String {
    format!("agentmux-remote-{version}-{target}")
}

const HELPER_FILE: &str = "agentmux-remote";

/// Where a package keeps the build for `target`, relative to srv's own folder
/// (`exe_dir`): `tools/remote/<target>/` beside the other bundled tools, or, in
/// a macOS app, `../Resources/remote/<target>/`, because the bundle seal only
/// allows Mach-O code under `Contents/MacOS` and the Linux builds are data there.
pub fn packaged_paths(exe_dir: &Path, target: &str) -> [PathBuf; 2] {
    [
        exe_dir.join("tools").join("remote").join(target).join(HELPER_FILE),
        exe_dir.join("..").join("Resources").join("remote").join(target).join(HELPER_FILE),
    ]
}

/// The paths to try for `target`, in order: `dev_dir`
/// (`AGENTMUX_REMOTE_HELPER_DIR`) alone when it is set, else this package's.
fn candidate_paths(dev_dir: Option<&Path>, exe_dir: Option<&Path>, version: &str, target: &str) -> Vec<PathBuf> {
    match (dev_dir, exe_dir) {
        (Some(dir), _) => vec![
            dir.join(target).join(HELPER_FILE),
            dir.join(asset_name(version, target)),
        ],
        (None, Some(exe_dir)) => packaged_paths(exe_dir, target).to_vec(),
        (None, None) => Vec::new(),
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// The helper's folder on the host, relative to the remote home.
fn remote_dir(version: &str) -> String {
    format!("$HOME/.agentmux-remote/bin/{version}")
}

/// One remote command: `sh -c '<script>'`, single-quoted for whatever shell
/// the user logs in with.
fn sh(script: &str) -> String {
    format!("sh -c {}", super::ssh::quote(script))
}

/// Prints `uname -sm`, then this version's helper's `version` line if it is
/// installed and runs.
pub fn probe_command(version: &str) -> String {
    let dir = remote_dir(version);
    sh(&format!(
        "uname -sm; p=\"{dir}/agentmux-remote\"; if [ -x \"$p\" ]; then \"$p\" version 2>/dev/null; fi; true"
    ))
}

/// Reads the binary from stdin into a temp file in an owner-only folder.
/// A temp file of this install's own (`tag`: the pane's session id, letters,
/// digits, `-` and `_`), so two panes installing on one host at once never
/// write, check or remove each other's upload.
fn temp_name(tag: &str) -> String {
    let tag: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!("agentmux-remote.{tag}.tmp")
}

pub fn upload_command(version: &str, tag: &str) -> String {
    let dir = remote_dir(version);
    let tmp = temp_name(tag);
    sh(&format!(
        "umask 077; mkdir -p \"{dir}\" && cat > \"{dir}/{tmp}\""
    ))
}

/// Checks the uploaded file's hash; on a match makes it the helper, else
/// removes it. Prints `ok` or `mismatch <hash>`.
pub fn install_command(version: &str, tag: &str, sha256: &str) -> String {
    let dir = remote_dir(version);
    let tmp = temp_name(tag);
    sh(&format!(
        "f=\"{dir}/{tmp}\"; \
         h=$( (sha256sum \"$f\" 2>/dev/null || shasum -a 256 \"$f\") | cut -d' ' -f1); \
         if [ \"$h\" = \"{sha256}\" ]; then chmod 700 \"$f\" && mv -f \"$f\" \"{dir}/agentmux-remote\" && echo ok; \
         else rm -f \"$f\"; echo \"mismatch $h\"; fi"
    ))
}

/// What a probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub uname: String,
    /// This version's helper is installed and answered `version`.
    pub installed: bool,
}

/// Read a probe's output.
pub fn parse_probe(out: &str, version: &str) -> Probe {
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let uname = lines.next().unwrap_or_default().to_string();
    let want = format!(
        "agentmux-remote {version} protocol {}",
        agentmux_remote::frame::PROTOCOL
    );
    let installed = lines.any(|l| l == want);
    Probe { uname, installed }
}

/// This version's build for `target`, and its hash: from
/// `AGENTMUX_REMOTE_HELPER_DIR` when set (a developer's own builds), else from
/// this package (spec §9.1). Nothing is downloaded.
pub fn local_build(version: &str, target: &str) -> Result<(Vec<u8>, String), String> {
    let dev_dir = std::env::var_os("AGENTMUX_REMOTE_HELPER_DIR").map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    read_first(&candidate_paths(dev_dir.as_deref(), exe_dir.as_deref(), version, target), target)
}

fn read_first(paths: &[PathBuf], target: &str) -> Result<(Vec<u8>, String), String> {
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            let hash = sha256_hex(&bytes);
            return Ok((bytes, hash));
        }
    }
    Err(format!(
        "this AgentMux build doesn't include the helper for {target} (looked in {})",
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Put this version's helper on `host` if it is not there (spec §6.2): probe
/// the platform, get the matching build (checked against the release's
/// hashes), upload it over ssh, check its hash there, move it in. `tag` names
/// this install's own temp file (letters, digits, `-`, `_`), so two installs
/// on one host never touch each other's upload. `announce` hears the size
/// just before the upload, for whoever is waiting to say what is happening.
pub async fn ensure<F, Fut>(
    host: &super::host::HostSsh,
    tag: &str,
    announce: F,
) -> Result<(), String>
where
    F: FnOnce(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let run = |cmd: String, input: Option<Vec<u8>>| async move {
        host.run(&cmd, input, std::time::Duration::from_secs(120))
            .await?
            .ok()
    };
    let version = env!("CARGO_PKG_VERSION");
    let probe = parse_probe(&run(probe_command(version), None).await?, version);
    if probe.installed {
        return Ok(());
    }
    let target = target_for(&probe.uname).ok_or_else(|| {
        format!(
            "AgentMux has no helper build for this host ({})",
            probe.uname.trim()
        )
    })?;
    let (bytes, hash) = local_build(version, target)?;
    announce(bytes.len()).await;
    run(upload_command(version, tag), Some(bytes)).await?;
    let out = run(install_command(version, tag, &hash), None).await?;
    if out.trim() != "ok" {
        return Err(format!(
            "the uploaded helper did not check out on the host ({})",
            out.trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_supported_platform_has_its_build() {
        assert_eq!(
            target_for("Linux x86_64"),
            Some("x86_64-unknown-linux-musl")
        );
        assert_eq!(
            target_for("Linux aarch64\n"),
            Some("aarch64-unknown-linux-musl")
        );
        assert_eq!(target_for("Darwin arm64"), Some("aarch64-apple-darwin"));
        assert_eq!(target_for("Darwin x86_64"), Some("x86_64-apple-darwin"));
        assert_eq!(target_for("FreeBSD amd64"), None);
        assert_eq!(target_for("Linux armv7l"), None);
        assert_eq!(target_for(""), None);
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_package_carries_the_helper_beside_its_other_tools() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("runtime");
        let t = "x86_64-unknown-linux-musl";
        put(&exe_dir.join("tools").join("remote").join(t).join("agentmux-remote"), b"linux build");
        let (bytes, hash) = read_first(&candidate_paths(None, Some(&exe_dir), "0.59.9", t), t).unwrap();
        assert_eq!(bytes, b"linux build");
        assert_eq!(hash, sha256_hex(b"linux build"));
    }

    #[test]
    fn a_macos_app_keeps_the_linux_builds_in_its_resources() {
        let dir = tempfile::tempdir().unwrap();
        let contents = dir.path().join("AgentMux.app").join("Contents");
        let exe_dir = contents.join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let t = "aarch64-unknown-linux-musl";
        put(&contents.join("Resources").join("remote").join(t).join("agentmux-remote"), b"arm build");
        let (bytes, _) = read_first(&candidate_paths(None, Some(&exe_dir), "0.59.9", t), t).unwrap();
        assert_eq!(bytes, b"arm build");
    }

    #[test]
    fn a_developers_folder_wins_in_either_layout() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("runtime");
        let dev = dir.path().join("dev");
        let t = "x86_64-apple-darwin";
        put(&exe_dir.join("tools").join("remote").join(t).join("agentmux-remote"), b"packaged");
        put(&dev.join(t).join("agentmux-remote"), b"dev, script layout");
        let (bytes, _) = read_first(&candidate_paths(Some(&dev), Some(&exe_dir), "0.59.9", t), t).unwrap();
        assert_eq!(bytes, b"dev, script layout");

        let flat = dir.path().join("flat");
        put(&flat.join(asset_name("0.59.9", t)), b"dev, flat name");
        let (bytes, _) = read_first(&candidate_paths(Some(&flat), Some(&exe_dir), "0.59.9", t), t).unwrap();
        assert_eq!(bytes, b"dev, flat name");
    }

    #[test]
    fn a_build_without_the_helper_says_so_and_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let t = "aarch64-apple-darwin";
        let err = read_first(&candidate_paths(None, Some(dir.path()), "0.59.9", t), t).unwrap_err();
        assert!(err.contains("doesn't include the helper for aarch64-apple-darwin"), "{err}");
        assert!(!err.contains("http"), "{err}");
    }

    #[test]
    fn a_probe_reads_the_platform_and_whether_this_version_is_there() {
        let v = "0.59.8";
        let ok = format!(
            "Linux x86_64\nagentmux-remote {v} protocol {}\n",
            agentmux_remote::frame::PROTOCOL
        );
        assert_eq!(
            parse_probe(&ok, v),
            Probe {
                uname: "Linux x86_64".into(),
                installed: true
            }
        );
        assert!(!parse_probe("Linux x86_64\n", v).installed);
        assert!(
            !parse_probe("Linux x86_64\nagentmux-remote 0.59.7 protocol 1\n", v).installed,
            "another version"
        );
    }

    #[test]
    fn the_remote_scripts_run_under_sh_whatever_the_login_shell() {
        for cmd in [
            probe_command("0.59.8"),
            upload_command("0.59.8", "amx-1"),
            install_command("0.59.8", "amx-1", &"a".repeat(64)),
        ] {
            assert!(cmd.starts_with("sh -c '"), "{cmd}");
            assert!(cmd.ends_with('\''), "{cmd}");
            assert!(cmd.contains("$HOME/.agentmux-remote/bin/0.59.8"), "{cmd}");
        }
        assert!(install_command("0.59.8", "amx-1", "abc").contains("= \"abc\" ]"));
        // Each install its own temp file; nothing but the id's own characters.
        assert!(upload_command("0.59.8", "amx-1").contains("agentmux-remote.amx-1.tmp"));
        assert!(upload_command("0.59.8", "a;b $x").contains("agentmux-remote.abx.tmp"));
    }

    /// The scripts do what they say in a real `sh` (where there is one).
    #[cfg(unix)]
    #[test]
    fn upload_then_install_puts_a_checked_binary_in_place() {
        let home = tempfile::tempdir().unwrap();
        let run = |cmd: &str, input: &[u8]| {
            use std::io::Write;
            let mut child = std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .env("HOME", home.path())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input).unwrap();
            String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
        };
        let body = b"#!/bin/sh\necho agentmux-remote 0.59.8 protocol 1\n";
        run(&upload_command("0.59.8", "s1"), body);
        assert!(run(&install_command("0.59.8", "s1", &"0".repeat(64)), b"").starts_with("mismatch"));
        // Another pane's upload in flight is untouched by this one's install.
        run(&upload_command("0.59.8", "s2"), b"other");
        run(&upload_command("0.59.8", "s1"), body);
        assert_eq!(
            run(&install_command("0.59.8", "s1", &sha256_hex(body)), b"").trim(),
            "ok"
        );
        assert!(home
            .path()
            .join(".agentmux-remote/bin/0.59.8/agentmux-remote.s2.tmp")
            .exists());
        let probe = parse_probe(&run(&probe_command("0.59.8"), b""), "0.59.8");
        assert!(probe.installed, "{probe:?}");
    }
}
