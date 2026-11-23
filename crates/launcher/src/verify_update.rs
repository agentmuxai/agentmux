// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-launcher --verify-update <version> <dir> <file> [installed:<version>] [unreachable:<why>]`
//!
//! Used by the root helper of a `.deb` or `.rpm` install (`agentmux-install-update`,
//! `docs/specs/SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md` §3.2): the
//! helper fetches the release's envelope, signature and manifest into `<dir>`
//! (named as `agentmux_common::update_manifest::file_names` names them; a file the
//! release doesn't have is left out) and asks this to check `<file>` against them with
//! the key this build pins, the same decision the app makes before it downloads
//! (`SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md`). One pinned key and one decision, so
//! the shell script holds neither.
//!
//! With `installed:<version>` it also holds the release to its upgrade floor
//! (`SPEC_UPGRADE_FLOOR_IN_SIGNED_MANIFEST`): the higher of the floor the verified
//! manifest signs and this build's own, against the version the helper read from the
//! package database. The helper runs as root on a caller's word, so it takes the floor
//! from the manifest it verified and the installed version from the database, never from
//! the caller; a release whose manifest signs no floor is checked as before.
//!
//! Exit status: 0 verified; 10 the release predates signing and has no manifest (the
//! helper falls back to the checksum GitHub lists, as the app does); 1 refused, with the
//! message the person reads on stderr.

use std::io::Read;
use std::path::Path;

use agentmux_common::update_manifest::{
    decide, file_names, install_first_message, judge_file_and_floor, signed_floor_in_force, version_core, Outcome, SignedFloor, UpdateKey,
    MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS,
};
use sha2::{Digest, Sha256};

/// Exit status for a release made before signing existed, with no manifest.
pub const EXIT_UNSIGNED_OLD: i32 = 10;

/// What a check allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The signed manifest lists the file with its size and hash.
    Verified,
    /// A release from before signing, which has no manifest.
    UnsignedOld,
}

/// A file a release publishes, if it was fetched, read with a cap.
fn read_capped(path: &Path, cap: usize) -> Result<Option<Vec<u8>>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("can't read {}: {e}", path.display())),
    };
    let mut body = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut body).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    if body.len() > cap {
        return Err(format!("{} is larger than {cap} bytes", path.display()));
    }
    Ok(Some(body))
}

/// The size and SHA-256 of `path`.
fn size_and_hash(path: &Path) -> Result<(u64, String), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, agentmux_common::update_manifest::hex(&hasher.finalize())))
}

/// The upgrade floor of the repository when this build was made (`UPGRADE_FLOOR`): a floor only rises, so no later
/// release's is lower. The app's check holds the same value.
const BUILD_FLOOR: &str = include_str!("../../../UPGRADE_FLOOR");

/// [`BUILD_FLOOR`] when it is a version.
fn build_floor() -> Option<&'static str> {
    Some(BUILD_FLOOR.trim()).filter(|f| version_core(f).is_some())
}

/// Check `file` against the signed manifest files in `dir` for `version`, using `keys`.
/// `unreachable` is the helper's reason when it couldn't fetch them at all. `installed` is the version the
/// package database says is installed and about to be replaced: given, the release's upgrade floor is enforced.
pub fn verify(keys: &[UpdateKey], version: &str, dir: &Path, file: &Path, unreachable: Option<&str>) -> Result<Verdict, String> {
    verify_replacing(keys, build_floor(), version, dir, file, unreachable, None)
}

/// [`verify`], holding the release to its upgrade floor against `installed` when that is given. `build` is this build's own floor.
pub fn verify_replacing(
    keys: &[UpdateKey],
    build: Option<&str>,
    version: &str,
    dir: &Path,
    file: &Path,
    unreachable: Option<&str>,
    installed: Option<&str>,
) -> Result<Verdict, String> {
    let tag = format!("v{}", version.trim_start_matches('v'));
    let name = file.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{} has no file name", file.display()))?;
    let mut signed_floor = SignedFloor::Absent;
    let outcome = if let Some(why) = unreachable {
        Outcome::Unreachable(why.to_string())
    } else {
        let (envelope_name, signature_name, manifest_name) = file_names(version.trim_start_matches('v'));
        match read_capped(&dir.join(envelope_name), MAX_ENVELOPE_BYTES)? {
            // The release has no signed manifest (made before signing).
            None => Outcome::Unsigned,
            Some(envelope) => match read_capped(&dir.join(signature_name), MAX_ENVELOPE_BYTES)? {
                None => Outcome::Failed("the envelope has no signature file".into()),
                Some(signatures) => match read_capped(&dir.join(manifest_name), MAX_MANIFEST_BYTES)? {
                    None => Outcome::Failed("the envelope has no manifest".into()),
                    Some(manifest) => {
                        let (size, sha256) = size_and_hash(file)?;
                        let (outcome, floor) = judge_file_and_floor(keys, &tag, Some((&envelope, &String::from_utf8_lossy(&signatures), &manifest)), name, size, &sha256);
                        signed_floor = floor;
                        outcome
                    }
                },
            },
        }
    };
    decide(version, &outcome)?;
    // Only a verified manifest has a floor (`signed_floor` is `Absent` for anything else). The installed version is read by
    // the helper from the package database; one that can't be read as a version, beside a floor, is refused.
    if let (Some(installed), Some(floor)) = (installed, signed_floor_in_force(build, &signed_floor)) {
        let older = match (version_core(installed), version_core(&floor)) {
            (Some(i), Some(f)) => i < f,
            _ => true,
        };
        if older {
            return Err(install_first_message(installed, version.trim_start_matches('v'), &floor));
        }
    }
    Ok(if outcome == Outcome::Unsigned { Verdict::UnsignedOld } else { Verdict::Verified })
}

/// What `--verify-update` was asked: the version, the folder of signed files, the package, and the optional parts.
#[derive(Debug, PartialEq, Eq)]
struct Request<'a> {
    version: &'a str,
    dir: &'a str,
    file: &'a str,
    /// The version the package database says is installed (`installed:<version>`).
    installed: Option<&'a str>,
    /// Why the helper could not fetch the signed files (`unreachable:<why>`), in one string.
    unreachable: Option<String>,
}

/// Read the words after `--verify-update`. `installed:<version>` comes before the reason, which is one argument whose
/// words were split apart and are put back together.
fn parse_request(rest: &[String]) -> Option<Request<'_>> {
    let (version, dir, file) = (rest.first()?, rest.get(1)?, rest.get(2)?);
    let (installed, reason_from) = match rest.get(3).and_then(|a| a.strip_prefix("installed:")) {
        Some(v) => (Some(v), 4),
        None => (None, 3),
    };
    let reason = rest[reason_from..].join(" ");
    let unreachable = reason.strip_prefix("unreachable:").map(str::to_string);
    Some(Request { version, dir, file, installed, unreachable })
}

/// Handle `--verify-update` if it is what was asked, returning the exit status.
pub fn run_if_requested(args: &[String]) -> Option<i32> {
    let at = args.iter().position(|a| a == "--verify-update")?;
    let Some(request) = parse_request(&args[at + 1..]) else {
        eprintln!("usage: agentmux-launcher --verify-update <version> <dir> <file> [installed:<version>] [unreachable:<why>]");
        return Some(2);
    };
    let result = verify_replacing(
        UPDATE_KEYS,
        build_floor(),
        request.version,
        Path::new(request.dir),
        Path::new(request.file),
        request.unreachable.as_deref(),
        request.installed,
    );
    Some(match result {
        Ok(Verdict::Verified) => 0,
        Ok(Verdict::UnsignedOld) => EXIT_UNSIGNED_OLD,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real v0.60.4 release's signed files, as published (the pinned key `k1` signed them).
    const ENVELOPE: &str = include_str!("../tests/fixtures/verify_update/AgentMux-0.60.4-envelope.json");
    const SIGNATURES: &str = include_str!("../tests/fixtures/verify_update/AgentMux-0.60.4-envelope.json.sig");
    const MANIFEST: &str = include_str!("../tests/fixtures/verify_update/AgentMux-0.60.4-manifest.json");

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("verify-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn signed_dir(name: &str) -> std::path::PathBuf {
        let dir = scratch(name);
        let (e, s, m) = file_names("0.60.4");
        std::fs::write(dir.join(e), ENVELOPE).unwrap();
        std::fs::write(dir.join(s), SIGNATURES).unwrap();
        std::fs::write(dir.join(m), MANIFEST).unwrap();
        dir
    }

    /// A file the real manifest lists, made to have a given content, is not the real one:
    /// the manifest's own entry is used to build a stand-in only where a test needs it.
    fn put(dir: &Path, name: &str, body: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn the_real_v0_60_4_manifest_verifies_with_the_pinned_key_and_refuses_a_file_that_is_not_listed() {
        let dir = signed_dir("real");
        let signed = agentmux_common::update_manifest::verify_release(UPDATE_KEYS, "v0.60.4", ENVELOPE.as_bytes(), SIGNATURES, MANIFEST.as_bytes())
            .expect("the published v0.60.4 files verify with the pinned key");
        assert!(signed.file("AgentMux_0.60.4_amd64.deb").is_some());
        // Not the real package: the manifest lists the name, and the size and hash differ.
        let fake = put(&dir, "AgentMux_0.60.4_amd64.deb", b"not the real package");
        assert!(verify(UPDATE_KEYS, "0.60.4", &dir, &fake, None).unwrap_err().contains("doesn't match"));
        // A name the signed manifest doesn't list: only a verified manifest gets this far.
        let other = put(&dir, "AgentMux_0.60.4_arm64.deb", b"x");
        assert!(verify(UPDATE_KEYS, "0.60.4", &dir, &other, None).unwrap_err().contains("no file for this kind of install"));
    }

    #[test]
    fn a_missing_envelope_is_unsigned_which_only_a_release_before_0_60_4_may_be() {
        let dir = scratch("unsigned");
        let f = put(&dir, "AgentMux_0.60.3_amd64.deb", b"x");
        assert_eq!(verify(UPDATE_KEYS, "0.60.3", &dir, &f, None), Ok(Verdict::UnsignedOld));
        let f = put(&dir, "AgentMux_0.60.4_amd64.deb", b"x");
        assert!(verify(UPDATE_KEYS, "0.60.4", &dir, &f, None).unwrap_err().contains("wasn't signed"));
        // A version that can't be read is not an old one.
        assert!(verify(UPDATE_KEYS, "nightly", &dir, &f, None).unwrap_err().contains("wasn't signed"));
    }

    #[test]
    fn a_release_that_could_not_be_reached_is_refused_whatever_its_age() {
        let dir = scratch("unreachable");
        let f = put(&dir, "AgentMux_0.60.3_amd64.deb", b"x");
        let e = verify(UPDATE_KEYS, "0.60.3", &dir, &f, Some("GitHub didn't answer")).unwrap_err();
        assert!(e.contains("Couldn't check the signature of v0.60.3") && e.contains("GitHub didn't answer"), "{e}");
    }

    #[test]
    fn a_tampered_manifest_a_missing_signature_and_another_tag_are_refused() {
        let (e, s, m) = file_names("0.60.4");
        let dir = signed_dir("tamper");
        let f = put(&dir, "AgentMux_0.60.4_amd64.deb", b"x");
        std::fs::write(dir.join(&m), MANIFEST.replace("\"size\"", "\"size \"")).unwrap();
        assert!(verify(UPDATE_KEYS, "0.60.4", &dir, &f, None).unwrap_err().contains("signature on v0.60.4 doesn't match"));
        let dir = signed_dir("nosig");
        std::fs::remove_file(dir.join(&s)).unwrap();
        let f = put(&dir, "AgentMux_0.60.4_amd64.deb", b"x");
        assert!(verify(UPDATE_KEYS, "0.60.4", &dir, &f, None).unwrap_err().contains("signature on v0.60.4 doesn't match"));
        // The envelope of 0.60.4 offered for 0.60.5 under that release's names.
        let dir = scratch("replay");
        std::fs::write(dir.join("AgentMux-0.60.5-envelope.json"), ENVELOPE).unwrap();
        std::fs::write(dir.join("AgentMux-0.60.5-envelope.json.sig"), SIGNATURES).unwrap();
        std::fs::write(dir.join("AgentMux-0.60.5-manifest.json"), MANIFEST).unwrap();
        let f = put(&dir, "AgentMux_0.60.5_amd64.deb", b"x");
        assert!(verify(UPDATE_KEYS, "0.60.5", &dir, &f, None).unwrap_err().contains("signature on v0.60.5 doesn't match"));
        let _ = e;
    }

    #[test]
    fn a_file_the_manifest_lists_with_its_size_and_hash_is_verified() {
        // A manifest of our own under a throwaway key stands in for "the right file".
        use agentmux_common::update_manifest::hex;
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        let body = b"the package bytes";
        let sha = hex(&Sha256::digest(body));
        let manifest = format!(r#"{{"format":1,"version":"0.60.9","tag":"v0.60.9","files":[{{"name":"AgentMux_0.60.9_amd64.deb","size":{},"sha256":"{sha}"}}]}}"#, body.len());
        let envelope = format!(r#"{{"format":1,"tag":"v0.60.9","manifest_sha256":"{}","created_at":"2026-10-28T12:00:00Z"}}"#, hex(&Sha256::digest(manifest.as_bytes())));
        let key = SigningKey::from_bytes(&[5; 32]);
        let sig = format!("k1 {}\n", base64::engine::general_purpose::STANDARD.encode(key.sign(envelope.as_bytes()).to_bytes()));
        let keys = [UpdateKey { id: "k1", public: key.verifying_key().to_bytes() }];
        let dir = scratch("good");
        let (e, s, m) = file_names("0.60.9");
        std::fs::write(dir.join(e), &envelope).unwrap();
        std::fs::write(dir.join(s), &sig).unwrap();
        std::fs::write(dir.join(m), &manifest).unwrap();
        let f = put(&dir, "AgentMux_0.60.9_amd64.deb", body);
        assert_eq!(verify(&keys, "0.60.9", &dir, &f, None), Ok(Verdict::Verified));
        // One byte different, same name: refused.
        let f = put(&dir, "AgentMux_0.60.9_amd64.deb", b"the package byteZ");
        assert!(verify(&keys, "0.60.9", &dir, &f, None).is_err());
        // The throwaway key isn't the one the app pins.
        assert!(verify(UPDATE_KEYS, "0.60.9", &dir, &f, None).is_err());
    }

    /// A release of our own under a throwaway key whose manifest signs `upgrade_json` (e.g. `,"upgrade":{"floor":"0.58.0"}`),
    /// with the package it lists: the keys, the folder and the package's path.
    fn floor_release(name: &str, upgrade_json: &str) -> ([UpdateKey; 1], std::path::PathBuf, std::path::PathBuf) {
        use agentmux_common::update_manifest::hex;
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        let body = b"the package bytes";
        let sha = hex(&Sha256::digest(body));
        let manifest = format!(
            r#"{{"format":1,"version":"0.60.9","tag":"v0.60.9","files":[{{"name":"AgentMux_0.60.9_amd64.deb","size":{},"sha256":"{sha}"}}]{upgrade_json}}}"#,
            body.len()
        );
        let envelope = format!(r#"{{"format":1,"tag":"v0.60.9","manifest_sha256":"{}","created_at":"2026-10-28T12:00:00Z"}}"#, hex(&Sha256::digest(manifest.as_bytes())));
        let key = SigningKey::from_bytes(&[5; 32]);
        let sig = format!("k1 {}\n", base64::engine::general_purpose::STANDARD.encode(key.sign(envelope.as_bytes()).to_bytes()));
        let dir = scratch(name);
        let (e, s, m) = file_names("0.60.9");
        std::fs::write(dir.join(e), &envelope).unwrap();
        std::fs::write(dir.join(s), &sig).unwrap();
        std::fs::write(dir.join(m), &manifest).unwrap();
        let f = put(&dir, "AgentMux_0.60.9_amd64.deb", body);
        ([UpdateKey { id: "k1", public: key.verifying_key().to_bytes() }], dir, f)
    }

    #[test]
    fn a_release_whose_signed_floor_is_above_the_installed_version_is_refused_naming_the_version_to_install_first() {
        let (keys, dir, f) = floor_release("floor-above", r#","upgrade":{"floor":"0.58.0"}"#);
        let e = verify_replacing(&keys, Some("0.55.0"), "0.60.9", &dir, &f, None, Some("0.57.3")).unwrap_err();
        assert_eq!(e, "This AgentMux (0.57.3) is older than 0.60.9 can upgrade from; install 0.58.0 first, then upgrade.");
        // The app says the same words for the same case.
        assert_eq!(e, install_first_message("0.57.3", "0.60.9", "0.58.0"));
    }

    #[test]
    fn at_or_above_the_signed_floor_the_release_installs_as_before() {
        let (keys, dir, f) = floor_release("floor-at", r#","upgrade":{"floor":"0.58.0"}"#);
        for installed in ["0.58.0", "0.58.1", "0.60.8", "0.60.8-1"] {
            assert_eq!(verify_replacing(&keys, Some("0.55.0"), "0.60.9", &dir, &f, None, Some(installed)), Ok(Verdict::Verified), "{installed}");
        }
    }

    #[test]
    fn a_manifest_with_no_usable_floor_is_checked_as_before_whatever_is_installed() {
        for (name, upgrade) in [("none", ""), ("empty", r#","upgrade":{}"#), ("above", r#","upgrade":{"floor":"0.99.0"}"#), ("junk", r#","upgrade":{"floor":"soon"}"#)] {
            let (keys, dir, f) = floor_release(&format!("nofloor-{name}"), upgrade);
            assert_eq!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, Some("0.1.0")), Ok(Verdict::Verified), "{name}");
        }
    }

    #[test]
    fn the_higher_of_the_signed_floor_and_this_builds_floor_holds_and_a_signed_floor_never_lowers_it() {
        let (keys, dir, f) = floor_release("floor-build", r#","upgrade":{"floor":"0.55.0"}"#);
        let e = verify_replacing(&keys, Some("0.59.0"), "0.60.9", &dir, &f, None, Some("0.58.0")).unwrap_err();
        assert!(e.contains("install 0.59.0 first"), "{e}");
        assert_eq!(verify_replacing(&keys, Some("0.59.0"), "0.60.9", &dir, &f, None, Some("0.59.0")), Ok(Verdict::Verified));
    }

    #[test]
    fn an_installed_version_that_cannot_be_read_is_refused_beside_a_floor_and_ignored_without_one() {
        let (keys, dir, f) = floor_release("floor-unreadable", r#","upgrade":{"floor":"0.58.0"}"#);
        assert!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, Some("nightly")).is_err());
        assert!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, Some("")).is_err());
        let (keys, dir, f) = floor_release("nofloor-unreadable", "");
        assert_eq!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, Some("nightly")), Ok(Verdict::Verified));
    }

    #[test]
    fn a_floor_is_read_only_from_a_manifest_that_verifies_and_never_asked_for_without_an_installed_version() {
        // Going back to an older release passes no installed version: the floor is not asked.
        let (keys, dir, f) = floor_release("floor-goback", r#","upgrade":{"floor":"0.58.0"}"#);
        assert_eq!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, None), Ok(Verdict::Verified));
        // A floor lowered after signing breaks the signature: refused whatever is installed, and nothing is read from it.
        let manifest = std::fs::read_to_string(dir.join(file_names("0.60.9").2)).unwrap();
        std::fs::write(dir.join(file_names("0.60.9").2), manifest.replace("0.58.0", "0.10.0")).unwrap();
        assert!(verify_replacing(&keys, None, "0.60.9", &dir, &f, None, Some("0.57.0")).unwrap_err().contains("signature on v0.60.9 doesn't match"));
    }

    #[test]
    fn the_installed_version_is_read_before_the_reason_and_the_reason_keeps_its_words() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let r = a(&["0.60.9", "/d", "/f", "installed:0.57.3", "unreachable:GitHub", "didn't", "answer"]);
        let q = parse_request(&r).unwrap();
        assert_eq!((q.version, q.dir, q.file, q.installed), ("0.60.9", "/d", "/f", Some("0.57.3")));
        assert_eq!(q.unreachable.as_deref(), Some("GitHub didn't answer"));
        let r = a(&["0.60.9", "/d", "/f"]);
        assert_eq!(parse_request(&r).unwrap().installed, None);
        // The old form, a reason alone, is still read.
        let r = a(&["0.60.9", "/d", "/f", "unreachable:x"]);
        let q = parse_request(&r).unwrap();
        assert_eq!((q.installed, q.unreachable.as_deref()), (None, Some("x")));
        assert!(parse_request(&a(&["0.60.9", "/d"])).is_none());
    }

    #[test]
    fn the_verb_is_found_among_the_arguments_and_a_short_one_is_a_usage_error() {
        assert_eq!(run_if_requested(&["agentmux-launcher".into()]), None);
        assert_eq!(run_if_requested(&["agentmux-launcher".into(), "--verify-update".into(), "0.60.4".into()]), Some(2));
    }
}
