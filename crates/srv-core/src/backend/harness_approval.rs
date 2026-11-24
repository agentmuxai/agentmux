// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a person approved when they registered a custom harness: the manifest and the program it
//! runs, each pinned by hash. A harness may start only while both still match, so a changed
//! manifest or a swapped or updated binary asks again
//! (SPEC_BYO_HARNESS_CUSTOM_ACP_MANIFEST_2026_11_12.md §5, items 2 and 3).
//!
//! Pure logic and files in the harness's own folder: nothing here starts a program, and nothing
//! here decides who may approve (that is the `harness_register` sensitive operation).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::harness_manifest::{HarnessManifest, Install};
use super::providers_custom::manifest_hash;

/// The approval's file name, beside `harness.json`.
pub const APPROVAL_FILE: &str = "approval.json";

/// What was approved, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    /// SHA-256 of the manifest's canonical JSON ([`manifest_hash`]).
    pub manifest_hash: String,
    /// The program's absolute path as it resolved at approval.
    pub binary_path: String,
    /// SHA-256 of that file's bytes.
    pub binary_sha256: String,
    pub approved_at_ms: u64,
    /// Who approved it, as the sensitive operation recorded it (a person or an agent, by name).
    pub approved_by: String,
}

/// Why a harness may not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No approval is on record.
    NotApproved,
    /// The manifest changed since it was approved.
    ManifestChanged,
    /// The approved program is gone, or cannot be read.
    BinaryMissing(String),
    /// The program at the approved path is not the one that was approved.
    BinaryChanged { approved: String, found: String },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotApproved => write!(f, "this custom harness has not been approved; register it and approve it first"),
            Refusal::ManifestChanged => write!(f, "this custom harness's manifest changed since it was approved; approve it again"),
            Refusal::BinaryMissing(why) => write!(f, "the approved program cannot be read ({why}); register the harness again"),
            Refusal::BinaryChanged { approved, found } => write!(
                f,
                "the program of this custom harness changed since it was approved (approved {}, found {}); approve it again",
                short(approved),
                short(found)
            ),
        }
    }
}

impl std::error::Error for Refusal {}

fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// The approval on record for the harness whose folder is `dir`, if it reads.
pub fn read(dir: &Path) -> Option<Approval> {
    serde_json::from_slice(&std::fs::read(dir.join(APPROVAL_FILE)).ok()?).ok()
}

/// Record `approval` in `dir`: written to a temporary file and renamed, so a reader never sees half of it.
pub fn write(dir: &Path, approval: &Approval) -> std::io::Result<()> {
    let tmp = dir.join(format!("{APPROVAL_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(approval).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, dir.join(APPROVAL_FILE))
}

/// Forget an approval (a removal, or a person withdrawing it). Absent is fine.
pub fn clear(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(APPROVAL_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// SHA-256 of a file's bytes, read in blocks (a program can be large).
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hasher.update(&block[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The program a manifest with `install: path` names, as an absolute path to a file: the command as given when it
/// is absolute, else the first match on the PATH. An installed source (npm, download) has no path until its install
/// has run, so it is refused here.
pub fn resolve_command(manifest: &HarnessManifest) -> Result<PathBuf, String> {
    if !matches!(manifest.install, Install::Path) {
        return Err("a program installed by AgentMux is pinned after its install, not before".to_string());
    }
    let path = Path::new(&manifest.command);
    let found = if path.is_absolute() {
        path.to_path_buf()
    } else {
        which::which(&manifest.command).map_err(|_| format!("{} is not on the PATH", manifest.command))?
    };
    // Follow links, so the hash and the later start name the same file.
    let real = std::fs::canonicalize(&found).map_err(|e| format!("{}: {e}", found.display()))?;
    if real.is_file() {
        Ok(real)
    } else {
        Err(format!("{} is not a file", real.display()))
    }
}

/// An approval for `manifest` as it stands now, with the program `binary` hashed at this moment.
pub fn approval_for(manifest: &HarnessManifest, binary: &Path, approved_by: &str, now_ms: u64) -> std::io::Result<Approval> {
    Ok(Approval {
        manifest_hash: manifest_hash(manifest),
        binary_path: binary.to_string_lossy().into_owned(),
        binary_sha256: sha256_file(binary)?,
        approved_at_ms: now_ms,
        approved_by: approved_by.to_string(),
    })
}

/// Whether the harness may start: an approval is on record, for this manifest, and the program at the approved path
/// still hashes to what was approved. Run before every start.
pub fn check(manifest: &HarnessManifest, dir: &Path) -> Result<Approval, Refusal> {
    let approval = read(dir).ok_or(Refusal::NotApproved)?;
    if approval.manifest_hash != manifest_hash(manifest) {
        return Err(Refusal::ManifestChanged);
    }
    let found = sha256_file(Path::new(&approval.binary_path)).map_err(|e| Refusal::BinaryMissing(e.to_string()))?;
    if found != approval.binary_sha256 {
        return Err(Refusal::BinaryChanged { approved: approval.binary_sha256, found });
    }
    Ok(approval)
}

#[cfg(test)]
#[path = "harness_approval_tests.rs"]
mod tests;
