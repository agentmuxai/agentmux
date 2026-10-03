// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Installing AgentMux's helper (`agentmux-remote`) on an SSH host, for
//! durable sessions (spec §6.2 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! 1. Probe the host over the pane's own ssh: `uname -sm`, and whether this
//!    version's helper is already at `~/.agentmux-remote/bin/<version>/`.
//! 2. Get the matching static build: a local cache, or this version's GitHub
//!    release asset, checked against the release's SHA256SUMS. (A dev build
//!    can point `AGENTMUX_REMOTE_HELPER_DIR` at its own builds.)
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

/// The release asset name of a build: `agentmux-remote-<version>-<target>`.
pub fn asset_name(version: &str, target: &str) -> String {
    format!("agentmux-remote-{version}-{target}")
}

fn sums_name(version: &str) -> String {
    format!("agentmux-remote-{version}-SHA256SUMS")
}

fn release_url(version: &str, name: &str) -> String {
    format!("https://github.com/agentmuxai/agentmux/releases/download/v{version}/{name}")
}

/// The hash a SHA256SUMS file gives for `name`.
pub fn hash_in_sums(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
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

/// This version's build for `target`: from `AGENTMUX_REMOTE_HELPER_DIR` (a
/// dev build), the cache under `cache_dir`, or the release, its hash checked
/// against the release's SHA256SUMS before it is cached. Returns the bytes
/// and their hash.
pub async fn local_build(
    version: &str,
    target: &str,
    cache_dir: &Path,
    http: &reqwest::Client,
) -> Result<(Vec<u8>, String), String> {
    let name = asset_name(version, target);
    if let Some(dir) = std::env::var_os("AGENTMUX_REMOTE_HELPER_DIR") {
        let path = PathBuf::from(dir).join(&name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let hash = sha256_hex(&bytes);
        return Ok((bytes, hash));
    }
    let cached = cache_dir.join(&name);
    let cached_hash = cache_dir.join(format!("{name}.sha256"));
    if let (Ok(bytes), Ok(hash)) = (
        std::fs::read(&cached),
        std::fs::read_to_string(&cached_hash),
    ) {
        if sha256_hex(&bytes) == hash.trim() {
            return Ok((bytes, hash.trim().to_string()));
        }
    }
    let get = |url: String| async move {
        let resp = http
            .get(&url)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| format!("could not download {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("could not download {url}: HTTP {}", resp.status()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("could not download {url}: {e}"))
    };
    let sums = get(release_url(version, &sums_name(version))).await?;
    let want = hash_in_sums(&String::from_utf8_lossy(&sums), &name)
        .ok_or_else(|| format!("this release publishes no helper for {target}"))?;
    let bytes = get(release_url(version, &name)).await?;
    let got = sha256_hex(&bytes);
    if got != want {
        return Err(format!(
            "the downloaded helper's hash is {got}, not the release's {want}"
        ));
    }
    if std::fs::create_dir_all(cache_dir).is_ok() {
        let _ = std::fs::write(&cached, &bytes);
        let _ = std::fs::write(&cached_hash, &got);
    }
    Ok((bytes, got))
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

    #[test]
    fn a_sums_file_gives_each_builds_hash() {
        let h = "a".repeat(64);
        let sums = format!("{h}  agentmux-remote-0.59.8-x86_64-unknown-linux-musl\n{}  *agentmux-remote-0.59.8-aarch64-apple-darwin\n", "B".repeat(64));
        assert_eq!(
            hash_in_sums(&sums, "agentmux-remote-0.59.8-x86_64-unknown-linux-musl"),
            Some(h)
        );
        assert_eq!(
            hash_in_sums(&sums, "agentmux-remote-0.59.8-aarch64-apple-darwin"),
            Some("b".repeat(64))
        );
        assert_eq!(
            hash_in_sums(&sums, "agentmux-remote-0.59.8-x86_64-apple-darwin"),
            None
        );
        assert_eq!(hash_in_sums("nothex  x", "x"), None);
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
