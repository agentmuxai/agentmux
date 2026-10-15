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
const MIB: u64 = 1024 * 1024;

/// The most one file, and all of a jekt's files together, may weigh when
/// they travel on `tier` (spec §5): `(per file, per jekt)`. The same machine
/// (`host`, `channel`) allows the most; LAN less; anything else nothing yet.
pub fn tier_limits(tier: &str) -> Option<(u64, u64)> {
    match tier {
        "host" | "channel" => Some((100 * MIB, 200 * MIB)),
        "lan" => Some((50 * MIB, 100 * MIB)),
        _ => None,
    }
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

    #[test]
    fn tier_limits_are_tighter_further_away() {
        let f = |size| JektFile { sha256: "a".repeat(64), name: "a".into(), size };
        assert!(check_tier_limits(&[f(100 * MIB)], "channel").is_ok());
        assert!(check_tier_limits(&[f(100 * MIB)], "lan").is_err());
        assert!(check_tier_limits(&[f(40 * MIB), f(40 * MIB), f(40 * MIB)], "lan").is_err(), "per jekt");
        assert!(check_tier_limits(&[f(1)], "wan").is_err(), "not yet");
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
