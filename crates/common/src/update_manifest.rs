// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The signed manifest of a release's files
//! (`docs/specs/SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md`).
//!
//! A release publishes three small files beside its downloads: a **manifest**
//! (every file with its size and SHA-256), an **envelope** that names the
//! manifest by its hash and the release by its tag, and a **signature** of the
//! envelope's exact bytes. AgentMux pins the public key and takes a release
//! file's name, size and hash only from a manifest that [`verify_release`]
//! passed, so a file that didn't come out of the release pipeline (a swapped
//! asset, a rewritten API answer) is never offered, downloaded or run.
//!
//! This module only checks bytes it is given: fetching them, and what the
//! person is told, are the update check's (`srv::backend::update_check`).
//! The release workflow makes and verifies the same files with
//! `scripts/release-manifest.mjs`; `tests::a_release_made_by_the_script_verifies`
//! pins the two to one format.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The envelope's `format`, the only one understood.
pub const ENVELOPE_FORMAT: u32 = 1;
/// The manifest's `format`, the only one understood.
pub const MANIFEST_FORMAT: u32 = 1;
/// The most bytes read for an envelope: it is a fixed handful of fields.
pub const MAX_ENVELOPE_BYTES: usize = 4096;
/// The most bytes read for a manifest. It grows with the files of a release; a
/// megabyte is thousands of files.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
/// The most files a manifest may list.
const MAX_FILES: usize = 1000;

/// A public key the app trusts to sign releases, with the id signature lines
/// name it by.
#[derive(Debug, Clone, Copy)]
pub struct UpdateKey {
    pub id: &'static str,
    pub public: [u8; 32],
}

/// The keys this build trusts: at most two (the current one and, once a
/// rotation is prepared, the next; spec §6). `k1` is the release signing key
/// (plain Ed25519, raw 32-byte public key); the private half never leaves the
/// signing service.
///
/// `update-keys.json` (beside `src/`) lists the same keys in base64 for the
/// release workflow, and a test keeps the two equal.
pub const UPDATE_KEYS: &[UpdateKey] = &[UpdateKey {
    id: "k1",
    public: [
        0x9b, 0xd1, 0xd1, 0x67, 0x85, 0x0e, 0x88, 0x15, 0x36, 0xbf, 0x2a, 0xe3, 0x94, 0x75, 0xe3, 0xd4, 0x3d, 0x02, 0x8d, 0xf8, 0x40, 0x6b, 0x32, 0x36, 0x0c, 0x67, 0x48, 0x23, 0xd2, 0x52, 0x24, 0xb1,
    ],
}];

/// What is signed: a release's tag and the hash of its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Envelope {
    pub format: u32,
    pub tag: String,
    pub manifest_sha256: String,
    pub created_at: String,
}

/// One release file, as the pipeline made it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManifestFile {
    pub name: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// A release's files.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub tag: String,
    pub files: Vec<ManifestFile>,
}

impl Manifest {
    /// The release file called `name`, if the manifest lists it.
    pub fn file(&self, name: &str) -> Option<&ManifestFile> {
        self.files.iter().find(|f| f.name == name)
    }
}

/// Why a release's manifest isn't trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// No signature from a key this build trusts (a release made before
    /// signing, or one signed only with a key since retired).
    Unsigned,
    /// A signature names a trusted key, but it isn't a valid signature of the
    /// envelope.
    BadSignature,
    /// The envelope isn't what the format says.
    Envelope(String),
    /// The envelope is for another release than the one asked about.
    WrongTag { expected: String, got: String },
    /// The manifest isn't the one the envelope names.
    ManifestMismatch,
    /// The manifest isn't what the format says.
    Manifest(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Unsigned => write!(f, "the release isn't signed with a key this AgentMux trusts"),
            VerifyError::BadSignature => write!(f, "the release's signature doesn't match"),
            VerifyError::Envelope(why) => write!(f, "the release's envelope is invalid: {why}"),
            VerifyError::WrongTag { expected, got } => write!(f, "the envelope is for {got}, not {expected}"),
            VerifyError::ManifestMismatch => write!(f, "the manifest isn't the one the signature covers"),
            VerifyError::Manifest(why) => write!(f, "the release's manifest is invalid: {why}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A plain release file name: no path, nothing hidden, nothing a shell or
/// Windows would treat specially.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Whether one of `signatures`' lines is a valid signature of `envelope` by a
/// key in `keys`. `Err(Unsigned)` when no line names a trusted key at all.
///
/// A signature file is one line per signature: `<key id> <base64 Ed25519
/// signature>`, over the envelope file's exact bytes (spec §3). Lines for keys
/// this build doesn't pin are ignored, so a release signed by both the current
/// and the next key verifies on a build that knows only one.
fn check_signature(keys: &[UpdateKey], envelope: &[u8], signatures: &str) -> Result<(), VerifyError> {
    let mut named_a_trusted_key = false;
    for line in signatures.lines() {
        let mut parts = line.split_whitespace();
        let (Some(id), Some(sig_b64), None) = (parts.next(), parts.next(), parts.next()) else { continue };
        for key in keys.iter().filter(|k| k.id == id) {
            named_a_trusted_key = true;
            let Ok(verifying) = VerifyingKey::from_bytes(&key.public) else { continue };
            let Ok(sig) = BASE64.decode(sig_b64) else { continue };
            let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { continue };
            if verifying.verify(envelope, &Signature::from_bytes(&sig)).is_ok() {
                return Ok(());
            }
        }
    }
    Err(if named_a_trusted_key { VerifyError::BadSignature } else { VerifyError::Unsigned })
}

/// Check a release's `envelope`, its `signatures` file and its `manifest`
/// against the trusted `keys` for the release called `tag`, and return the
/// manifest only if every step passes (spec §5.2):
///
/// 1. a signature of the envelope by a trusted key;
/// 2. the envelope's format, and its tag is `tag` (so an old signed envelope
///    can't be replayed under a newer tag);
/// 3. the manifest's SHA-256 is the envelope's `manifest_sha256`;
/// 4. the manifest's format, tag and version agree with the envelope, and each
///    file in it has a plain name, a SHA-256 and a size.
pub fn verify_release(
    keys: &[UpdateKey],
    tag: &str,
    envelope: &[u8],
    signatures: &str,
    manifest: &[u8],
) -> Result<Manifest, VerifyError> {
    if envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(VerifyError::Envelope("too large".into()));
    }
    check_signature(keys, envelope, signatures)?;
    let env: Envelope = serde_json::from_slice(envelope).map_err(|e| VerifyError::Envelope(e.to_string()))?;
    if env.format != ENVELOPE_FORMAT {
        return Err(VerifyError::Envelope(format!("format {} isn't understood", env.format)));
    }
    if env.tag != tag {
        return Err(VerifyError::WrongTag { expected: tag.to_string(), got: env.tag });
    }
    if !is_sha256_hex(&env.manifest_sha256) {
        return Err(VerifyError::Envelope("manifest_sha256 isn't a SHA-256".into()));
    }
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(VerifyError::Manifest("too large".into()));
    }
    if !hex(&Sha256::digest(manifest)).eq_ignore_ascii_case(&env.manifest_sha256) {
        return Err(VerifyError::ManifestMismatch);
    }
    let parsed: Manifest = serde_json::from_slice(manifest).map_err(|e| VerifyError::Manifest(e.to_string()))?;
    if parsed.format != MANIFEST_FORMAT {
        return Err(VerifyError::Manifest(format!("format {} isn't understood", parsed.format)));
    }
    if parsed.tag != env.tag || parsed.version != env.tag.trim_start_matches('v') {
        return Err(VerifyError::Manifest("it names another release than the envelope".into()));
    }
    if parsed.files.is_empty() || parsed.files.len() > MAX_FILES {
        return Err(VerifyError::Manifest("the number of files is out of range".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &parsed.files {
        if !is_plain_file_name(&f.name) {
            return Err(VerifyError::Manifest(format!("{:?} isn't a plain file name", f.name)));
        }
        if !is_sha256_hex(&f.sha256) {
            return Err(VerifyError::Manifest(format!("{}: sha256 isn't a SHA-256", f.name)));
        }
        if f.size == 0 {
            return Err(VerifyError::Manifest(format!("{}: size is zero", f.name)));
        }
        if !seen.insert(f.name.as_str()) {
            return Err(VerifyError::Manifest(format!("{} is listed twice", f.name)));
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const TAG: &str = "v0.60.1";

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn key(id: &'static str, seed: u8) -> UpdateKey {
        UpdateKey { id, public: signing_key(seed).verifying_key().to_bytes() }
    }

    fn manifest_json(files: &[(&str, u64, &str)]) -> String {
        let files: Vec<String> =
            files.iter().map(|(n, s, h)| format!(r#"{{"name":"{n}","size":{s},"sha256":"{h}"}}"#)).collect();
        format!(r#"{{"format":1,"version":"0.60.1","tag":"{TAG}","files":[{}]}}"#, files.join(","))
    }

    fn envelope_json(tag: &str, manifest: &str) -> String {
        format!(
            r#"{{"format":1,"tag":"{tag}","manifest_sha256":"{}","created_at":"2026-10-25T12:00:00Z"}}"#,
            hex(&Sha256::digest(manifest.as_bytes()))
        )
    }

    fn sig_line(id: &str, seed: u8, envelope: &str) -> String {
        format!("{id} {}\n", BASE64.encode(signing_key(seed).sign(envelope.as_bytes()).to_bytes()))
    }

    const H1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const H2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn release() -> (String, String) {
        let manifest = manifest_json(&[("AgentMux-0.60.1-x64-setup.exe", 100, H1), ("agentmux-0.60.1-x64-portable.7z", 50, H2)]);
        (envelope_json(TAG, &manifest), manifest)
    }

    #[test]
    fn a_signed_release_gives_its_files() {
        let (env, man) = release();
        let m = verify_release(&[key("k1", 1)], TAG, env.as_bytes(), &sig_line("k1", 1, &env), man.as_bytes()).unwrap();
        assert_eq!(m.file("AgentMux-0.60.1-x64-setup.exe").map(|f| (f.size, f.sha256.as_str())), Some((100, H1)));
        assert_eq!(m.file("nope.exe"), None);
    }

    #[test]
    fn a_release_with_no_signature_from_a_trusted_key_is_unsigned() {
        let (env, man) = release();
        let keys = [key("k1", 1)];
        assert_eq!(verify_release(&keys, TAG, env.as_bytes(), "", man.as_bytes()), Err(VerifyError::Unsigned));
        // Signed by a key this build doesn't pin.
        assert_eq!(verify_release(&keys, TAG, env.as_bytes(), &sig_line("k9", 9, &env), man.as_bytes()), Err(VerifyError::Unsigned));
        // Nothing pinned yet (the shipped list): every release is unsigned.
        assert_eq!(verify_release(&[], TAG, env.as_bytes(), &sig_line("k1", 1, &env), man.as_bytes()), Err(VerifyError::Unsigned));
        assert!(UPDATE_KEYS.len() <= 2, "at most two keys: current and next");
    }

    #[test]
    fn a_signature_that_doesnt_match_is_refused() {
        let (env, man) = release();
        let keys = [key("k1", 1)];
        // The right key id with another key's signature.
        assert_eq!(verify_release(&keys, TAG, env.as_bytes(), &sig_line("k1", 2, &env), man.as_bytes()), Err(VerifyError::BadSignature));
        // A tampered envelope.
        let tampered = env.replace("2026-10-25", "2026-10-26");
        assert_eq!(
            verify_release(&keys, TAG, tampered.as_bytes(), &sig_line("k1", 1, &env), man.as_bytes()),
            Err(VerifyError::BadSignature)
        );
        // Garbage in the signature field.
        assert_eq!(verify_release(&keys, TAG, env.as_bytes(), "k1 !!!\n", man.as_bytes()), Err(VerifyError::BadSignature));
    }

    #[test]
    fn either_of_two_keys_may_have_signed_and_unknown_lines_are_ignored() {
        let (env, man) = release();
        let both = [key("k1", 1), key("k2", 2)];
        for sigs in [sig_line("k1", 1, &env), sig_line("k2", 2, &env), format!("k9 AAAA\n{}{}", sig_line("k1", 9, &env), sig_line("k2", 2, &env))] {
            assert!(verify_release(&both, TAG, env.as_bytes(), &sigs, man.as_bytes()).is_ok(), "{sigs}");
        }
        // A build that knows only the older key still takes a release signed by both.
        let both_sigs = format!("{}{}", sig_line("k1", 1, &env), sig_line("k2", 2, &env));
        assert!(verify_release(&[key("k1", 1)], TAG, env.as_bytes(), &both_sigs, man.as_bytes()).is_ok());
        // And one signed only by the next key is unsigned to it.
        assert_eq!(
            verify_release(&[key("k1", 1)], TAG, env.as_bytes(), &sig_line("k2", 2, &env), man.as_bytes()),
            Err(VerifyError::Unsigned)
        );
    }

    #[test]
    fn an_old_signed_envelope_cant_be_replayed_under_another_tag() {
        let (env, man) = release();
        let keys = [key("k1", 1)];
        let sigs = sig_line("k1", 1, &env);
        assert_eq!(
            verify_release(&keys, "v0.60.2", env.as_bytes(), &sigs, man.as_bytes()),
            Err(VerifyError::WrongTag { expected: "v0.60.2".into(), got: TAG.into() })
        );
    }

    #[test]
    fn a_manifest_other_than_the_one_signed_is_refused_however_large_it_grew() {
        let (env, man) = release();
        let keys = [key("k1", 1)];
        let sigs = sig_line("k1", 1, &env);
        let swapped = man.replace(H1, H2);
        assert_eq!(verify_release(&keys, TAG, env.as_bytes(), &sigs, swapped.as_bytes()), Err(VerifyError::ManifestMismatch));
        // A manifest well past the old 4 KB is fine when it is the signed one.
        let many: Vec<(String, u64, &str)> = (0..200).map(|i| (format!("file-{i}.bin"), 1 + i as u64, H1)).collect();
        let refs: Vec<(&str, u64, &str)> = many.iter().map(|(n, s, h)| (n.as_str(), *s, *h)).collect();
        let big = manifest_json(&refs);
        assert!(big.len() > 8192);
        let env2 = envelope_json(TAG, &big);
        assert_eq!(verify_release(&keys, TAG, env2.as_bytes(), &sig_line("k1", 1, &env2), big.as_bytes()).unwrap().files.len(), 200);
    }

    #[test]
    fn a_manifest_that_breaks_the_format_is_refused_even_when_signed() {
        let keys = [key("k1", 1)];
        let check = |manifest: String| {
            let env = envelope_json(TAG, &manifest);
            verify_release(&keys, TAG, env.as_bytes(), &sig_line("k1", 1, &env), manifest.as_bytes())
        };
        let ok = |name: &str| check(manifest_json(&[(name, 1, H1)]));
        assert!(ok("AgentMux-0.60.1-x64-setup.exe").is_ok());
        for bad in ["../evil.exe", "a/b.exe", "a\\b.exe", ".hidden", "", "has space.exe", "semi;colon.exe", "dots..exe"] {
            assert!(matches!(ok(bad), Err(VerifyError::Manifest(_))), "{bad:?}");
        }
        assert!(matches!(check(manifest_json(&[("a.exe", 0, H1)])), Err(VerifyError::Manifest(_))), "size zero");
        assert!(matches!(check(manifest_json(&[("a.exe", 1, "ABC")])), Err(VerifyError::Manifest(_))), "bad hash");
        assert!(matches!(check(manifest_json(&[("a.exe", 1, &"A".repeat(64))])), Err(VerifyError::Manifest(_))), "uppercase hash");
        assert!(matches!(check(manifest_json(&[("a.exe", 1, H1), ("a.exe", 2, H2)])), Err(VerifyError::Manifest(_))), "twice");
        assert!(matches!(check(manifest_json(&[])), Err(VerifyError::Manifest(_))), "no files");
        assert!(matches!(check(manifest_json(&[("a.exe", 1, H1)]).replace(r#""format":1"#, r#""format":2"#)), Err(VerifyError::Manifest(_))));
        assert!(matches!(check(manifest_json(&[("a.exe", 1, H1)]).replace("0.60.1", "0.60.9")), Err(VerifyError::Manifest(_))), "another version");
        assert!(matches!(check("not json".into()), Err(VerifyError::Manifest(_))));
    }

    #[test]
    fn an_envelope_that_breaks_the_format_is_refused_even_when_signed() {
        let keys = [key("k1", 1)];
        let (_, man) = release();
        let sha = hex(&Sha256::digest(man.as_bytes()));
        let check = |env: String| verify_release(&keys, TAG, env.as_bytes(), &sig_line("k1", 1, &env), man.as_bytes());
        let good = format!(r#"{{"format":1,"tag":"{TAG}","manifest_sha256":"{sha}","created_at":"x"}}"#);
        assert!(check(good.clone()).is_ok());
        assert!(matches!(check(good.replace(r#""format":1"#, r#""format":2"#)), Err(VerifyError::Envelope(_))));
        assert!(matches!(check(good.replace(&sha, "abc")), Err(VerifyError::Envelope(_))));
        assert!(matches!(check("{}".into()), Err(VerifyError::Envelope(_))));
        let huge = format!(r#"{{"format":1,"tag":"{TAG}","manifest_sha256":"{sha}","created_at":"{}"}}"#, "x".repeat(5000));
        assert!(matches!(check(huge), Err(VerifyError::Envelope(_))), "an envelope is small by definition");
    }

    // A release as `scripts/release-manifest.mjs` makes it (build, then sign-key with a
    // local key whose seed is thirty-two 7s), copied here byte for byte: the script
    // that signs in the release workflow and this code that checks in the app agree
    // on every byte, or this fails. The same release is built and checked on the
    // script's side in `scripts/release-manifest.test.mjs`.
    #[test]
    fn a_release_made_by_the_script_verifies() {
        const PUBLIC_KEY_B64: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
        const SIGNATURES: &str = "k1 oGZeVKdahQJP1mqd964PNT7Fie6IbSoUMkDXrmi1QbUVn947HuHtRZNRJt36L22rcAkLPGRWolco8DB2AZXZCw==
";
        const ENVELOPE: &str = "{
  \"format\": 1,
  \"tag\": \"v0.60.1\",
  \"manifest_sha256\": \"8a83f0bb7ce99e7537b872b55d6e688da6713ebd078b0dd88c740b1a3c5809eb\",
  \"created_at\": \"2026-10-25T12:00:00Z\"
}
";
        const MANIFEST: &str = "{
  \"format\": 1,
  \"version\": \"0.60.1\",
  \"tag\": \"v0.60.1\",
  \"files\": [
    {
      \"name\": \"AgentMux-0.60.1-x64-setup.exe\",
      \"size\": 100,
      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"
    },
    {
      \"name\": \"agentmux-0.60.1-x64-portable.7z\",
      \"size\": 50,
      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"
    }
  ]
}
";
        let public: [u8; 32] = BASE64.decode(PUBLIC_KEY_B64).unwrap().try_into().unwrap();
        let keys = [UpdateKey { id: "k1", public }];
        let m = verify_release(&keys, "v0.60.1", ENVELOPE.as_bytes(), SIGNATURES, MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.file("agentmux-0.60.1-x64-portable.7z").map(|f| f.size), Some(50));
        // One flipped byte anywhere is refused.
        let flipped = MANIFEST.replace("\"size\": 50", "\"size\": 51");
        assert_eq!(verify_release(&keys, "v0.60.1", ENVELOPE.as_bytes(), SIGNATURES, flipped.as_bytes()), Err(VerifyError::ManifestMismatch));
    }

    // The release workflow reads `update-keys.json` to check what it signed; the app
    // pins `UPDATE_KEYS`. They are the same keys or the workflow checks the wrong thing.
    #[test]
    fn the_keys_file_the_workflow_reads_is_the_keys_the_app_pins() {
        #[derive(Deserialize)]
        struct File {
            keys: Vec<FileKey>,
        }
        #[derive(Deserialize)]
        struct FileKey {
            id: String,
            public_key: String,
        }
        let file: File = serde_json::from_str(include_str!("../update-keys.json")).unwrap();
        assert_eq!(file.keys.len(), UPDATE_KEYS.len());
        for (f, k) in file.keys.iter().zip(UPDATE_KEYS) {
            assert_eq!(f.id, k.id);
            assert_eq!(BASE64.decode(&f.public_key).unwrap(), k.public);
        }
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
