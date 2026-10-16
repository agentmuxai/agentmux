// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files sent with a jekt: the manifest, its digest, and how the msgid binds
//! it (docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §3.2–§3.3).
//!
//! A jekt with files has a msgid ending in `.files-v1.<manifest digest>`.
//! Every signing scheme already signs the msgid, so the signature covers the
//! file list, and through each entry's SHA-256 every byte of every file,
//! without any scheme changing. A text-only jekt is untouched.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// At most this many files on one jekt.
pub const MAX_FILES: usize = 10;
/// What an instance that can receive files advertises, in its registry
/// entries and its `GET /agentmux/reactive/agent` answer (spec §3.4).
pub const FEATURE: &str = "files-v1";
/// What an install that can fetch files sent through the cloud relay says in
/// its agents' leases, which a sender asks about before sending (§4.4).
pub const CLOUD_FEATURE: &str = "files-cloud-v1";
const MIB: u64 = 1024 * 1024;

/// The most one file, and all of a jekt's files together, may weigh when
/// they travel on `tier` (spec §5): `(per file, per jekt)`. The same machine
/// (`host`, `channel`) allows the most; LAN less; anything else nothing yet.
pub fn tier_limits(tier: &str) -> Option<(u64, u64)> {
    match tier {
        "host" | "channel" => Some((100 * MIB, 200 * MIB)),
        "lan" => Some((50 * MIB, 100 * MIB)),
        "wan" => Some((25 * MIB, 50 * MIB)),
        _ => None,
    }
}

/// A file as a cloud jekt's manifest carries it (spec §4.4): the file, the
/// key its blob is encrypted under, and the relay's name for the blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFile {
    #[serde(flatten)]
    pub file: JektFile,
    /// Standard base64 of a 32-byte AES-256-GCM key, or that key sealed to
    /// the receiving install ([`is_sealed_key`], spec §7).
    pub key: String,
    pub blob_id: String,
}

/// Leads a cloud manifest's digest: never the same input as a plain one.
const CLOUD_DIGEST_LABEL: &str = "amx-jekt-files-cloud-v1";

/// SHA-256 over a cloud manifest, keys and blob ids included, so the WAN
/// signature that binds it covers them too.
pub fn cloud_manifest_digest(files: &[CloudFile]) -> String {
    let mut input = String::from(CLOUD_DIGEST_LABEL);
    for f in files {
        input.push(ENTRY_SEP);
        input.push_str(&f.file.sha256);
        for part in [f.file.name.as_str(), &f.file.size.to_string(), &f.key, &f.blob_id] {
            input.push(FIELD_SEP);
            input.push_str(part);
        }
    }
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

/// The msgid a WAN jekt carrying `files` is signed under: `msgid` without
/// any digest it already ended in, then the cloud manifest's digest.
pub fn bind_cloud_msgid(msgid: &str, files: &[CloudFile]) -> String {
    let base = match bound_digest(msgid) {
        Some(_) => &msgid[..msgid.rfind(MSGID_TAG).unwrap_or(msgid.len())],
        None => msgid,
    };
    format!("{base}{MSGID_TAG}{}", cloud_manifest_digest(files))
}

/// Whether `msgid` binds exactly this cloud manifest.
pub fn cloud_msgid_binds(msgid: &str, files: &[CloudFile]) -> bool {
    bound_digest(msgid).is_some_and(|d| d == cloud_manifest_digest(files))
}

/// Leads a file key sealed to its receiver: `x25519.<the receiving agent's
/// WAN key fingerprint>.<base64url of the ephemeral key, the wrapped key and
/// its tag>` (spec §7).
pub const SEALED_KEY_PREFIX: &str = "x25519.";
/// Base64url length of a sealed key's last part: 32 + 32 + 16 bytes.
const SEALED_KEY_BODY_CHARS: usize = 107;

/// Whether `key` has a sealed key's shape. Says nothing about whether it opens.
pub fn is_sealed_key(key: &str) -> bool {
    let b64url = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    key.strip_prefix(SEALED_KEY_PREFIX)
        .and_then(|rest| rest.split_once('.'))
        .is_some_and(|(fp, body)| fp.len() == 43 && body.len() == SEALED_KEY_BODY_CHARS && b64url(fp) && b64url(body))
}

/// Why a cloud manifest is refused: the plain checks, plus a 44-character
/// base64 key (or a sealed one) and a relay blob id on every entry.
pub fn validate_cloud(files: &[CloudFile]) -> Result<(), String> {
    let plain: Vec<JektFile> = files.iter().map(|f| f.file.clone()).collect();
    validate(&plain)?;
    let blob_id = |s: &str| s.len() == 41 && s.starts_with("blob-") && s[5..].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    let key = |k: &str| (k.len() == 44 && k.ends_with('=')) || is_sealed_key(k);
    if files.iter().any(|f| !key(&f.key) || !blob_id(&f.blob_id)) {
        return Err("a cloud file entry is malformed".to_string());
    }
    Ok(())
}

/// Why `files` can't travel on `tier`, if they can't.
pub fn check_tier_limits(files: &[JektFile], tier: &str) -> Result<(), String> {
    let Some((per_file, per_jekt)) = tier_limits(tier) else {
        return Err(format!("files can't be sent over the {tier} tier yet"));
    };
    if let Some(f) = files.iter().find(|f| f.size > per_file) {
        return Err(format!("{} is over the {} MB limit per file on this tier", f.name, per_file / MIB));
    }
    if files.iter().map(|f| f.size).sum::<u64>() > per_jekt {
        return Err(format!("the files come to more than the {} MB limit per message on this tier", per_jekt / MIB));
    }
    Ok(())
}
/// A file name's length limit, in characters (the attachment store's).
pub const MAX_NAME_CHARS: usize = 120;
/// Separates the msgid a sender would otherwise use from the digest.
pub const MSGID_TAG: &str = ".files-v1.";
/// Leads the digest input, so it can't be read as any other signed material.
const DIGEST_LABEL: &str = "amx-jekt-files-v1";
const FIELD_SEP: char = '\u{1}';
const ENTRY_SEP: char = '\u{2}';

/// One file in a jekt's manifest, in send order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JektFile {
    /// Lowercase hex SHA-256 of the file's bytes.
    pub sha256: String,
    /// Base name only, as [`clean_name`] leaves it.
    pub name: String,
    pub size: u64,
}

/// A file's name as a manifest carries it: the base name of `name`, with
/// control characters replaced, trimmed, and at most [`MAX_NAME_CHARS`]
/// characters. `"file"` when nothing is left.
pub fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let one_line: String = base.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let short: String = one_line.trim().chars().take(MAX_NAME_CHARS).collect();
    let short = short.trim();
    if short.is_empty() || short == "." || short == ".." {
        "file".to_string()
    } else {
        short.to_string()
    }
}

/// Why a manifest is refused before anything else is checked.
pub fn validate(files: &[JektFile]) -> Result<(), String> {
    if files.is_empty() {
        return Err("the file list is empty".to_string());
    }
    if files.len() > MAX_FILES {
        return Err(format!("{} files is more than the limit of {MAX_FILES}", files.len()));
    }
    for f in files {
        let hex = f.sha256.len() == 64 && f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            return Err(format!("{}: not a lowercase SHA-256", clean_name(&f.name)));
        }
        if f.name != clean_name(&f.name) {
            return Err(format!("{}: not a clean base name", clean_name(&f.name)));
        }
        if f.size == 0 {
            return Err(format!("{}: empty file", f.name));
        }
    }
    Ok(())
}

/// Lowercase hex SHA-256 over the label and the entries, in order (§3.2).
pub fn manifest_digest(files: &[JektFile]) -> String {
    let mut input = String::from(DIGEST_LABEL);
    for f in files {
        input.push(ENTRY_SEP);
        input.push_str(&f.sha256);
        input.push(FIELD_SEP);
        input.push_str(&f.name);
        input.push(FIELD_SEP);
        input.push_str(&f.size.to_string());
    }
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

/// The msgid for a jekt carrying `files`: `base` with the digest appended.
pub fn bind_msgid(base: &str, files: &[JektFile]) -> String {
    format!("{base}{MSGID_TAG}{}", manifest_digest(files))
}

/// The digest a msgid says its jekt was sent with, if it says one.
pub fn bound_digest(msgid: &str) -> Option<&str> {
    let at = msgid.rfind(MSGID_TAG)?;
    let digest = &msgid[at + MSGID_TAG.len()..];
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digest)
}

/// Whether `msgid` binds exactly this manifest.
pub fn msgid_binds(msgid: &str, files: &[JektFile]) -> bool {
    bound_digest(msgid).is_some_and(|d| d == manifest_digest(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(n: u8, name: &str, size: u64) -> JektFile {
        JektFile { sha256: format!("{:x}", Sha256::digest([n])), name: name.to_string(), size }
    }

    #[test]
    fn binding_round_trips() {
        let files = vec![file(1, "a.png", 10), file(2, "b.txt", 20)];
        let msgid = bind_msgid("1792-42-0", &files);
        assert!(msgid.starts_with("1792-42-0.files-v1."));
        assert!(msgid_binds(&msgid, &files));
    }

    #[test]
    fn any_change_to_the_manifest_breaks_the_binding() {
        let files = vec![file(1, "a.png", 10), file(2, "b.txt", 20)];
        let msgid = bind_msgid("m", &files);
        let mut renamed = files.clone();
        renamed[0].name = "c.png".into();
        let mut resized = files.clone();
        resized[1].size = 21;
        let mut swapped = files.clone();
        swapped[0].sha256 = file(3, "", 0).sha256;
        let reordered = vec![files[1].clone(), files[0].clone()];
        for other in [renamed, resized, swapped, reordered, files[..1].to_vec()] {
            assert!(!msgid_binds(&msgid, &other), "{other:?}");
        }
    }

    #[test]
    fn entry_fields_cannot_shift_into_each_other() {
        // A separator inside a name would let two manifests share a digest.
        let a = vec![JektFile { sha256: "a".repeat(64), name: "x".into(), size: 1 }];
        let b = vec![JektFile { sha256: "a".repeat(64), name: "x\u{1}1".into(), size: 1 }];
        assert_ne!(manifest_digest(&a), manifest_digest(&b));
        assert!(validate(&b).is_err(), "control characters never reach a manifest");
    }

    #[test]
    fn a_text_msgid_binds_nothing() {
        for msgid in ["1792-42-0", "", "abc.files-v1.", "abc.files-v1.zz", &format!("x.files-v1.{}", "g".repeat(64))] {
            assert_eq!(bound_digest(msgid), None, "{msgid}");
        }
    }

    #[test]
    fn clean_name_keeps_only_a_short_base_name() {
        assert_eq!(clean_name("C:\\shots\\a.png"), "a.png");
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("a\nb\u{7}.txt"), "a b .txt");
        assert_eq!(clean_name("dir/"), "file");
        assert_eq!(clean_name(".."), "file");
        assert_eq!(clean_name(&"n".repeat(300)).chars().count(), MAX_NAME_CHARS);
    }

    fn cloud(n: u8) -> CloudFile {
        CloudFile {
            file: file(n, "a.png", 10),
            key: "A".repeat(43) + "=",
            blob_id: "blob-0123abcd-0000-4000-8000-0123456789ab".into(),
        }
    }

    #[test]
    fn a_cloud_manifest_binds_its_keys_and_blob_ids() {
        let files = vec![cloud(1)];
        let msgid = bind_cloud_msgid(&bind_msgid("m-1", &[cloud(1).file]), &files);
        assert!(msgid.starts_with("m-1.files-v1."), "the plain digest is replaced, not stacked: {msgid}");
        assert!(cloud_msgid_binds(&msgid, &files));
        let mut other_key = files.clone();
        other_key[0].key = "B".repeat(43) + "=";
        let mut other_blob = files.clone();
        other_blob[0].blob_id = "blob-ffffffff-0000-4000-8000-0123456789ab".into();
        assert!(!cloud_msgid_binds(&msgid, &other_key));
        assert!(!cloud_msgid_binds(&msgid, &other_blob));
        assert!(!msgid_binds(&msgid, &[cloud(1).file]), "a plain manifest never matches a cloud digest");
        assert!(validate_cloud(&files).is_ok());
        let mut bad = files.clone();
        bad[0].key = "short".into();
        assert!(validate_cloud(&bad).is_err());
    }

    #[test]
    fn a_sealed_key_passes_only_in_its_exact_shape() {
        let sealed = format!("{SEALED_KEY_PREFIX}{}.{}", "F_-".repeat(14) + "x", "g".repeat(SEALED_KEY_BODY_CHARS));
        assert!(is_sealed_key(&sealed));
        let mut files = vec![cloud(1)];
        files[0].key = sealed.clone();
        assert!(validate_cloud(&files).is_ok());
        for bad in [
            sealed[..sealed.len() - 1].to_string(),
            sealed.replace("x25519.", "x448."),
            format!("{sealed}="),
            sealed.replacen('g', "+", 1),
            sealed.replacen('_', ".", 1),
        ] {
            assert!(!is_sealed_key(&bad), "{bad}");
            files[0].key = bad;
            assert!(validate_cloud(&files).is_err());
        }
    }

    #[test]
    fn tier_limits_are_tighter_further_away() {
        let f = |size| JektFile { sha256: "a".repeat(64), name: "a".into(), size };
        assert!(check_tier_limits(&[f(100 * MIB)], "channel").is_ok());
        assert!(check_tier_limits(&[f(100 * MIB)], "lan").is_err());
        assert!(check_tier_limits(&[f(40 * MIB), f(40 * MIB), f(40 * MIB)], "lan").is_err(), "per jekt");
        assert!(check_tier_limits(&[f(30 * MIB)], "wan").is_err(), "cloud is tightest");
        assert!(check_tier_limits(&[f(1)], "elsewhere").is_err());
    }

    #[test]
    fn validate_refuses_bad_manifests() {
        assert!(validate(&[]).is_err());
        assert!(validate(&vec![file(1, "a", 1); MAX_FILES + 1]).is_err());
        assert!(validate(&[JektFile { sha256: "A".repeat(64), name: "a".into(), size: 1 }]).is_err());
        assert!(validate(&[file(1, "dir/a", 1)]).is_err());
        assert!(validate(&[file(1, "a", 0)]).is_err());
        assert!(validate(&[file(1, "a.png", 1)]).is_ok());
    }
}
