// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bring a skill in from outside the catalog: a local folder, a git repository over HTTPS, or a
//! single `SKILL.md` at an HTTPS URL. This module only reads and parses; it never writes the
//! catalog. The caller shows the whole text (and a diff against what the catalog holds), and
//! writes only what the person approved.
//!
//! Limits are deliberate: HTTPS only (a git `ext::` or `file://` source would run or read
//! something local), no symlinks followed, a size cap per file and a count cap per source.

use agentmux_common::win32::NoWindow;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One `SKILL.md` found in a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Where in the source it sits (`""` for the source's own root file), the stable key an update
    /// matches on.
    pub key: String,
    pub name: String,
    pub description: String,
    /// The body (the file without its frontmatter): what `db_skills.content` holds.
    pub content: String,
}

/// What a scan found, with the reasons anything was left out.
#[derive(Debug, Default)]
pub struct Scan {
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
    /// More skills were found than [`MAX_CANDIDATES`]; the rest were left out.
    capped: bool,
}

const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_CANDIDATES: usize = 100;
const MAX_DEPTH: usize = 4;
const GIT_TIMEOUT: Duration = Duration::from_secs(90);
const URL_TIMEOUT: Duration = Duration::from_secs(20);

/// Split a `SKILL.md` into name, description and body. Tolerant of what skills in the wild carry:
/// quoted or bare values, a folded or literal block for the description, extra frontmatter fields
/// (ignored) and CRLF line ends. Without frontmatter the whole text is the body.
pub fn parse_skill_md(text: &str, fallback_name: &str) -> (String, String, String) {
    let text = text.replace("\r\n", "\n");
    let Some(rest) = text.strip_prefix("---\n") else {
        return (fallback_name.to_string(), String::new(), text);
    };
    let (front, body) = match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            (&rest[..end], after.trim_start_matches('\n'))
        }
        None => return (fallback_name.to_string(), String::new(), text),
    };
    let lines: Vec<&str> = front.lines().collect();
    let (mut name, mut description) = (String::new(), String::new());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let mut collect_block = |style: &str| {
            let folded = style.starts_with('>');
            let mut parts: Vec<&str> = Vec::new();
            while i < lines.len() && (lines[i].starts_with(' ') || lines[i].starts_with('\t') || lines[i].trim().is_empty()) {
                parts.push(lines[i].trim());
                i += 1;
            }
            let parts: Vec<&str> = parts.into_iter().filter(|p| !p.is_empty()).collect();
            parts.join(if folded { " " } else { "\n" })
        };
        let is_block = value.starts_with('>') || value.starts_with('|');
        let value = if is_block { collect_block(value) } else { unquote(value) };
        match key.trim() {
            "name" => name = value,
            "description" => description = value,
            _ => {}
        }
    }
    let name = if name.trim().is_empty() { fallback_name.to_string() } else { name.trim().to_string() };
    (name, description.trim().to_string(), body.to_string())
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        if let Ok(s) = serde_json::from_str::<String>(v) {
            return s;
        }
    }
    if v.len() >= 2 && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"'))) {
        return v[1..v.len() - 1].replace("''", "'");
    }
    v.to_string()
}

fn read_candidate(file: &Path, key: &str, fallback_name: &str, warnings: &mut Vec<String>) -> Option<Candidate> {
    match std::fs::metadata(file) {
        Ok(m) if m.len() > MAX_FILE_BYTES => {
            warnings.push(format!("{key}: larger than {} KB, skipped", MAX_FILE_BYTES / 1024));
            return None;
        }
        Err(e) => {
            warnings.push(format!("{key}: {e}"));
            return None;
        }
        _ => {}
    }
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("{key}: not readable as text ({e}), skipped"));
            return None;
        }
    };
    let (name, description, content) = parse_skill_md(&text, fallback_name);
    Some(Candidate { key: key.to_string(), name, description, content })
}

/// Every `SKILL.md` under `root` (a skill folder, a pack of them, or one markdown file). A folder
/// with its own `SKILL.md` is one skill and is not searched further. Symlinks are not followed.
pub fn scan_dir(root: &Path) -> Result<Scan, String> {
    let root_name = root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    scan_dir_named(root, &root_name)
}

/// [`scan_dir`] with the name a root-level `SKILL.md` without a `name` takes (the folder's own
/// name is no use when the folder is a scratch clone).
fn scan_dir_named(root: &Path, root_name: &str) -> Result<Scan, String> {
    let meta = std::fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut scan = Scan::default();
    if meta.file_type().is_symlink() {
        return Err(format!("{} is a symbolic link", root.display()));
    }
    if meta.is_file() {
        let stem = root.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let fallback = if stem.eq_ignore_ascii_case("skill") {
            root.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or(stem)
        } else {
            stem
        };
        scan.candidates.extend(read_candidate(root, "", &fallback, &mut scan.warnings));
        return Ok(scan);
    }
    walk(root, root, root_name, 0, &mut scan);
    if scan.capped {
        scan.warnings.push(format!("only the first {MAX_CANDIDATES} skills are shown; import the rest from a smaller folder"));
    }
    scan.candidates.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(scan)
}

fn walk(root: &Path, dir: &Path, dir_name: &str, depth: usize, scan: &mut Scan) {
    let skill_md = dir.join("SKILL.md");
    if std::fs::symlink_metadata(&skill_md).map(|m| m.is_file()).unwrap_or(false) {
        if scan.candidates.len() >= MAX_CANDIDATES {
            scan.capped = true;
            return;
        }
        let key = dir.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        scan.candidates.extend(read_candidate(&skill_md, &key, dir_name, &mut scan.warnings));
        return;
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| !matches!(p.file_name().and_then(|n| n.to_str()), Some(".git" | "node_modules" | "target")))
        .collect();
    dirs.sort();
    for sub in dirs {
        let name = sub.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        walk(root, &sub, &name, depth + 1, scan);
    }
}

/// Whether a connection to this address is refused: loopback, private, link-local, shared
/// (carrier-grade NAT), multicast, broadcast, unspecified and documentation ranges, with an
/// IPv4-mapped IPv6 address judged as the IPv4 address it carries.
fn blocked_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
                || o[0] >= 240
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => blocked_ip(IpAddr::V4(v4)),
            None => {
                let s = v6.segments();
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (s[0] & 0xfe00) == 0xfc00 // unique local
                    || (s[0] & 0xffc0) == 0xfe80 // link-local
                    || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64 can reach an IPv4 host
            }
        },
    }
}

/// A host name or address a skill may be fetched from: https, no credentials, and not a local
/// name or a refused address. Applied to the first URL and again on every redirect.
fn check_url(parsed: &url::Url) -> Result<(), String> {
    if parsed.scheme() != "https" {
        return Err("only https:// sources are accepted".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("a URL with a user name or password is not accepted".to_string());
    }
    match parsed.host() {
        None => Err("the URL has no host".to_string()),
        Some(url::Host::Domain(d)) if d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost") => {
            Err("a local address is not accepted".to_string())
        }
        Some(url::Host::Ipv4(ip)) if blocked_ip(ip.into()) => Err("a local address is not accepted".to_string()),
        Some(url::Host::Ipv6(ip)) if blocked_ip(ip.into()) => Err("a local address is not accepted".to_string()),
        _ => Ok(()),
    }
}

fn https_url(raw: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw.trim()).map_err(|e| format!("not a valid URL: {e}"))?;
    check_url(&parsed)?;
    Ok(parsed)
}

/// Resolves host names for the fetch and drops every refused address, so a public name that
/// points at a private address (or is switched to one between a check and the connection) cannot
/// be reached, on the first request or on a redirect.
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found: Vec<std::net::SocketAddr> = tokio::net::lookup_host((name.as_str(), 443)).await?.collect();
            let allowed: Vec<std::net::SocketAddr> = found.into_iter().filter(|a| !blocked_ip(a.ip())).collect();
            if allowed.is_empty() {
                let e: Box<dyn std::error::Error + Send + Sync> = "that name only points at local addresses".into();
                return Err(e);
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// One `SKILL.md` fetched from an HTTPS URL.
pub async fn fetch_url(raw: &str) -> Result<Scan, String> {
    let parsed = https_url(raw)?;
    let client = reqwest::Client::builder()
        .timeout(URL_TIMEOUT)
        .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                attempt.error("too many redirects")
            } else if let Err(why) = check_url(attempt.url()) {
                attempt.error(format!("redirected somewhere not accepted: {why}"))
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client.get(parsed.clone()).send().await.map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fetch failed: HTTP {}", resp.status()));
    }
    let too_big = || format!("the file is larger than {} KB", MAX_FILE_BYTES / 1024);
    if resp.content_length().map(|n| n > MAX_FILE_BYTES).unwrap_or(false) {
        return Err(too_big());
    }
    // Read in pieces and stop at the cap: a server that sends no length cannot fill memory.
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("fetch failed: {e}"))? {
        if (bytes.len() + chunk.len()) as u64 > MAX_FILE_BYTES {
            return Err(too_big());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text".to_string())?;
    let segments: Vec<&str> = parsed.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let last = segments.last().copied().unwrap_or("skill");
    let stem = last.strip_suffix(".md").unwrap_or(last);
    let fallback = if stem.eq_ignore_ascii_case("skill") && segments.len() >= 2 { segments[segments.len() - 2] } else { stem };
    let (name, description, content) = parse_skill_md(&text, fallback);
    Ok(Scan { candidates: vec![Candidate { key: String::new(), name, description, content }], warnings: vec![], capped: false })
}

fn valid_git_ref(r: &str) -> bool {
    !r.is_empty()
        && !r.starts_with('-')
        && !r.contains("..")
        && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// Bytes of a clone that stop it: a skill repository is text, so this is generous.
const MAX_CLONE_BYTES: u64 = 200 * 1024 * 1024;

/// Total size of the files under `dir` (symlinks not followed).
fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Shallow-clone an HTTPS git repository into a scratch folder, scan it, delete the folder. The
/// clone runs with every protocol but https switched off, never asks for a password, and is
/// stopped when it runs past the time or size limit.
pub async fn fetch_git(raw: &str, git_ref: &str) -> Result<Scan, String> {
    use tokio::io::AsyncReadExt;
    let parsed = https_url(raw)?;
    let git_ref = git_ref.trim();
    if !git_ref.is_empty() && !valid_git_ref(git_ref) {
        return Err("that branch or tag name is not accepted".to_string());
    }
    // git resolves the name itself, so look at what it will be given first and refuse a name that
    // points at a local address. (Redirects are off for the clone.)
    if let Some(url::Host::Domain(host)) = parsed.host() {
        let found = tokio::net::lookup_host((host, 443)).await.map_err(|e| format!("could not find {host}: {e}"))?;
        if found.into_iter().any(|a| blocked_ip(a.ip())) {
            return Err("that name points at a local address, which is not accepted".to_string());
        }
    }
    let dir = std::env::temp_dir().join(format!("agentmux-skill-import-{}", uuid::Uuid::new_v4()));
    let mut cmd = tokio::process::Command::new("git");
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.no_window()
        .kill_on_drop(true)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .args(["-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "http.followRedirects=false"])
        .args(["clone", "--depth", "1", "--single-branch", "--no-tags"]);
    if !git_ref.is_empty() {
        cmd.args(["--branch", git_ref]);
    }
    cmd.arg("--").arg(parsed.as_str()).arg(&dir);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let result = async {
        let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let said = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });
        let started = std::time::Instant::now();
        let status = loop {
            tokio::select! {
                status = child.wait() => break status.map_err(|e| format!("could not run git: {e}"))?,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let why = if started.elapsed() > GIT_TIMEOUT {
                        Some(format!("git clone did not finish in {} s", GIT_TIMEOUT.as_secs()))
                    } else if dir_bytes(&dir) > MAX_CLONE_BYTES {
                        Some(format!("the repository is larger than {} MB", MAX_CLONE_BYTES / 1024 / 1024))
                    } else {
                        None
                    };
                    if let Some(why) = why {
                        let _ = child.kill().await;
                        return Err(why);
                    }
                }
            }
        };
        if !status.success() {
            let said = said.await.unwrap_or_default();
            return Err(format!("git clone failed: {}", said.trim().lines().last().unwrap_or("")));
        }
        let repo = parsed.path_segments().and_then(|s| s.filter(|p| !p.is_empty()).last()).unwrap_or("skill");
        scan_dir_named(&dir, repo.strip_suffix(".git").unwrap_or(repo))
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
#[path = "skill_import_tests.rs"]
mod tests;
