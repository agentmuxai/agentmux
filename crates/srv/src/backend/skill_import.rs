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
    let root_name = root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    walk(root, root, &root_name, 0, &mut scan);
    scan.candidates.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(scan)
}

fn walk(root: &Path, dir: &Path, dir_name: &str, depth: usize, scan: &mut Scan) {
    if scan.candidates.len() >= MAX_CANDIDATES {
        return;
    }
    let skill_md = dir.join("SKILL.md");
    if std::fs::symlink_metadata(&skill_md).map(|m| m.is_file()).unwrap_or(false) {
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

fn https_url(raw: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw.trim()).map_err(|e| format!("not a valid URL: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("only https:// sources are accepted".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("a URL with a user name or password is not accepted".to_string());
    }
    match parsed.host() {
        None => return Err("the URL has no host".to_string()),
        Some(url::Host::Domain(d)) if d.eq_ignore_ascii_case("localhost") || d.ends_with(".localhost") => {
            return Err("a local address is not accepted".to_string())
        }
        Some(url::Host::Ipv4(ip)) if ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() => {
            return Err("a local address is not accepted".to_string())
        }
        Some(url::Host::Ipv6(ip)) if ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80 => {
            return Err("a local address is not accepted".to_string())
        }
        _ => {}
    }
    Ok(parsed)
}

/// One `SKILL.md` fetched from an HTTPS URL.
pub async fn fetch_url(raw: &str) -> Result<Scan, String> {
    let parsed = https_url(raw)?;
    let client = reqwest::Client::builder()
        .timeout(URL_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(parsed.clone()).send().await.map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fetch failed: HTTP {}", resp.status()));
    }
    if resp.content_length().map(|n| n > MAX_FILE_BYTES).unwrap_or(false) {
        return Err(format!("the file is larger than {} KB", MAX_FILE_BYTES / 1024));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("fetch failed: {e}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} KB", MAX_FILE_BYTES / 1024));
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| "the file is not UTF-8 text".to_string())?;
    let segments: Vec<&str> = parsed.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let last = segments.last().copied().unwrap_or("skill");
    let stem = last.strip_suffix(".md").unwrap_or(last);
    let fallback = if stem.eq_ignore_ascii_case("skill") && segments.len() >= 2 { segments[segments.len() - 2] } else { stem };
    let (name, description, content) = parse_skill_md(&text, fallback);
    Ok(Scan { candidates: vec![Candidate { key: String::new(), name, description, content }], warnings: vec![] })
}

fn valid_git_ref(r: &str) -> bool {
    !r.is_empty()
        && !r.starts_with('-')
        && !r.contains("..")
        && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// Shallow-clone an HTTPS git repository into a scratch folder, scan it, delete the folder. The
/// clone runs with every protocol but https switched off and never asks for a password.
pub async fn fetch_git(raw: &str, git_ref: &str) -> Result<Scan, String> {
    let parsed = https_url(raw)?;
    let git_ref = git_ref.trim();
    if !git_ref.is_empty() && !valid_git_ref(git_ref) {
        return Err("that branch or tag name is not accepted".to_string());
    }
    let dir = std::env::temp_dir().join(format!("agentmux-skill-import-{}", uuid::Uuid::new_v4()));
    let mut cmd = tokio::process::Command::new("git");
    cmd.no_window()
        .kill_on_drop(true)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .args(["-c", "protocol.allow=never", "-c", "protocol.https.allow=always"])
        .args(["clone", "--depth", "1", "--single-branch", "--no-tags"]);
    if !git_ref.is_empty() {
        cmd.args(["--branch", git_ref]);
    }
    cmd.arg("--").arg(parsed.as_str()).arg(&dir);
    let result = match tokio::time::timeout(GIT_TIMEOUT, cmd.output()).await {
        Err(_) => Err(format!("git clone did not finish in {} s", GIT_TIMEOUT.as_secs())),
        Ok(Err(e)) => Err(format!("could not run git: {e}")),
        Ok(Ok(out)) if !out.status.success() => {
            Err(format!("git clone failed: {}", String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("")))
        }
        Ok(Ok(_)) => scan_dir(&dir),
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
#[path = "skill_import_tests.rs"]
mod tests;
