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
//! person is told, are the update check's (`srv::backend::update::check`).
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
/// The most images a manifest may list.
const MAX_IMAGES: usize = 16;

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

/// A container image the release built, named by the digest of its content: whatever a
/// registry serves under a tag, the image at this digest is the one the pipeline made.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManifestImage {
    /// What the release calls it, e.g. `agent-base`.
    pub name: String,
    /// The repository, without a tag or digest, e.g. `ghcr.io/agentmuxai/agent-base`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// `sha256:` and lowercase hex: the digest of the image (a multi-platform image's index).
    pub digest: String,
}

/// A release's files, and the images it built. Older manifests have no `images`, and an app
/// older than the field ignores it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub tag: String,
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub images: Vec<ManifestImage>,
    /// What the release says about upgrading to it: `floor` (the oldest version that may upgrade straight to it) and
    /// `shared_store_changed` (whether it changes a shared store's schema). Optional, under format 1, and kept as the
    /// JSON the pipeline signed: a field that isn't usable is read as absent by [`Manifest::signed_floor`] and
    /// [`Manifest::shared_store_changed`], never a reason to refuse the files (`SPEC_UPGRADE_FLOOR_IN_SIGNED_MANIFEST`).
    /// An app older than the field ignores it.
    #[serde(default)]
    pub upgrade: Option<serde_json::Value>,
}

/// The upgrade floor a verified manifest carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedFloor {
    /// The manifest says nothing about a floor: today's behaviour (the build's floor, and the notes' or the floor file's).
    Absent,
    /// A plain `X.Y.Z` that is not above the release's own version.
    Valid(String),
    /// The manifest has a floor that can't be used, and why: treated as absent, and logged by the caller.
    Unusable(String),
}

/// Whether the release changes a shared store's schema (`upgrade.shared_store_changed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedStoreChanged {
    /// Not said: not known, treated as no warning.
    Absent,
    /// `true` or `false`.
    Known(bool),
    /// Present but not a boolean: treated as absent, and logged by the caller.
    Unusable,
}

/// `<digits>.<digits>.<digits>` and nothing else (each part at most nine digits).
pub fn is_plain_version(s: &str) -> bool {
    let mut parts = s.split('.');
    let number = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()));
    number(parts.next()) && number(parts.next()) && number(parts.next()) && parts.next().is_none()
}

impl Manifest {
    /// The floor this manifest signs for its release. It does not refuse anything by itself: the pipeline refuses to sign a
    /// bad floor, so an unusable one is a bug there, and the app falls back to the floor it would use without one.
    pub fn signed_floor(&self) -> SignedFloor {
        let Some(upgrade) = &self.upgrade else { return SignedFloor::Absent };
        let Some(object) = upgrade.as_object() else { return SignedFloor::Unusable("upgrade isn't an object".into()) };
        let Some(floor) = object.get("floor") else { return SignedFloor::Absent };
        let Some(floor) = floor.as_str().filter(|f| is_plain_version(f)) else {
            return SignedFloor::Unusable("upgrade.floor isn't a plain version".into());
        };
        match (version_core(floor), version_core(&self.version)) {
            (Some(f), Some(v)) if f > v => SignedFloor::Unusable(format!("upgrade.floor {floor} is above the release's own version {}", self.version)),
            (Some(_), _) => SignedFloor::Valid(floor.to_string()),
            (None, _) => SignedFloor::Unusable("upgrade.floor isn't a plain version".into()),
        }
    }

    /// `upgrade.shared_store_changed`, as the pipeline signed it.
    pub fn shared_store_changed(&self) -> SharedStoreChanged {
        match self.upgrade.as_ref().and_then(|u| u.as_object()).and_then(|o| o.get("shared_store_changed")) {
            None => SharedStoreChanged::Absent,
            Some(serde_json::Value::Bool(b)) => SharedStoreChanged::Known(*b),
            Some(_) => SharedStoreChanged::Unusable,
        }
    }

    /// The release file called `name`, if the manifest lists it.
    pub fn file(&self, name: &str) -> Option<&ManifestFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// The image the release lists as `name`, if it does.
    pub fn image(&self, name: &str) -> Option<&ManifestImage> {
        self.images.iter().find(|i| i.name == name)
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

fn is_sha256_digest(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(is_sha256_hex)
}

/// A name for an image: lowercase letters, digits, `.`, `-` and `_`, starting with a letter or digit.
fn is_image_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// A registry repository path, `host[:port]/path/...`, with no tag, digest or scheme.
fn is_image_repository(reference: &str) -> bool {
    let mut parts = reference.split('/');
    let host = parts.next().unwrap_or_default();
    let path: Vec<&str> = parts.collect();
    let component = |c: &str| {
        !c.is_empty() && c.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit()) && c.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_'))
    };
    let host_ok = match host.split_once(':') {
        Some((h, port)) => component(h) && !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()),
        None => component(host),
    };
    reference.len() <= 255 && host_ok && !path.is_empty() && path.iter().all(|c| component(c))
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
///    file in it has a plain name, a SHA-256 and a size; each image, if it lists
///    any, has a plain name, a repository and a `sha256:` digest, and no name twice.
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
    if parsed.images.len() > MAX_IMAGES {
        return Err(VerifyError::Manifest("the number of images is out of range".into()));
    }
    let mut seen_images = std::collections::HashSet::new();
    for i in &parsed.images {
        if !is_image_name(&i.name) {
            return Err(VerifyError::Manifest(format!("{:?} isn't a plain image name", i.name)));
        }
        if !is_image_repository(&i.reference) {
            return Err(VerifyError::Manifest(format!("{}: ref isn't an image repository", i.name)));
        }
        if !is_sha256_digest(&i.digest) {
            return Err(VerifyError::Manifest(format!("{}: digest isn't sha256: and 64 hex digits", i.name)));
        }
        if !seen_images.insert(i.name.as_str()) {
            return Err(VerifyError::Manifest(format!("image {} is listed twice", i.name)));
        }
    }
    Ok(parsed)
}

/// The three files of a signed release, as published next to its other files:
/// the envelope, its signature and the manifest.
pub fn file_names(version: &str) -> (String, String, String) {
    (
        format!("AgentMux-{version}-envelope.json"),
        format!("AgentMux-{version}-envelope.json.sig"),
        format!("AgentMux-{version}-manifest.json"),
    )
}

/// The first release the pipeline signs (v0.60.4). From it on, a release without a
/// verified signed manifest is not installed; an older one never had one.
pub const FIRST_SIGNED: (u64, u64, u64) = (0, 60, 4);

/// `major.minor.patch` out of a version or tag (`v0.59.27`, `0.59.27+g1a2b`),
/// or `None` when it doesn't start with three integers.
pub fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['+', '-']).next().unwrap_or(v);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// What the signed manifest said about a release file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The release publishes no signed manifest (made before signing).
    Unsigned,
    /// Signed and valid; the file is listed with the size and hash it has.
    Verified,
    /// Signed and valid, but the file isn't listed in it.
    NotListed(String),
    /// Signed and valid, but the file's size or hash differ from the manifest's.
    Disagrees(String),
    /// A manifest is there but doesn't verify (or isn't what the envelope signed).
    Failed(String),
    /// The signed files couldn't be fetched (GitHub didn't answer, an error status): nothing is known.
    Unreachable(String),
}

/// The decision, given what was fetched (`None`: the release has no signed
/// manifest) and the file called `name` that has `size` bytes and hashes to `sha256`.
pub fn judge_file(keys: &[UpdateKey], tag: &str, fetched: Option<(&[u8], &str, &[u8])>, name: &str, size: u64, sha256: &str) -> Outcome {
    judge_file_and_floor(keys, tag, fetched, name, size, sha256).0
}

/// [`judge_file`], and the upgrade floor the manifest signs. The floor is read from the same verified manifest the file is
/// judged against, so a download can refuse on it without trusting what an earlier check saw; it is `Absent` whenever the
/// manifest did not verify (the outcome says why).
pub fn judge_file_and_floor(keys: &[UpdateKey], tag: &str, fetched: Option<(&[u8], &str, &[u8])>, name: &str, size: u64, sha256: &str) -> (Outcome, SignedFloor) {
    let Some((envelope, signatures, manifest)) = fetched else {
        return (Outcome::Unsigned, SignedFloor::Absent);
    };
    let manifest: Manifest = match verify_release(keys, tag, envelope, signatures, manifest) {
        Ok(m) => m,
        Err(e) => return (Outcome::Failed(e.to_string()), SignedFloor::Absent),
    };
    let outcome = match manifest.file(name) {
        None => Outcome::NotListed(format!("{name} isn't in the signed manifest")),
        Some(f) if f.size != size => Outcome::Disagrees(format!("size {} in the manifest, {size} in the file", f.size)),
        Some(f) if !f.sha256.eq_ignore_ascii_case(sha256) => Outcome::Disagrees("the manifest's SHA-256 differs from the file's".into()),
        Some(_) => Outcome::Verified,
    };
    (outcome, manifest.signed_floor())
}

/// What to do about `outcome` for `version`: `Ok` to go on, or the message to show (spec §7).
/// The app's download and the Linux package helper say the same thing.
pub fn decide(version: &str, outcome: &Outcome) -> Result<(), String> {
    let v = format!("v{}", version.trim_start_matches('v'));
    let signature_wrong = format!("The signature on {v} doesn't match, so AgentMux won't download it. Don't install it; tell the AgentMux team.");
    match outcome {
        Outcome::Verified => Ok(()),
        Outcome::Unsigned => match version_core(version) {
            // Made before signing existed: used as before.
            Some(core) if core < FIRST_SIGNED => Ok(()),
            // A version that can't be read is not trusted to be an old one.
            _ => Err(format!("{v} wasn't signed, so AgentMux won't download it itself. Get it from the release page.")),
        },
        Outcome::NotListed(_) => Err(format!("{v} has no file for this kind of install.")),
        Outcome::Disagrees(_) | Outcome::Failed(_) => Err(signature_wrong),
        Outcome::Unreachable(why) => Err(format!("Couldn't check the signature of {v} ({why}). Try again.")),
    }
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

    const D1: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const BASE: &str = "ghcr.io/agentmuxai/agent-base";

    fn with_images(images: &str) -> String {
        manifest_json(&[("a.exe", 1, H1)]).replace(r#""files":"#, &format!(r#""images":{images},"files":"#))
    }

    fn verify_with_images(images: &str) -> Result<Manifest, VerifyError> {
        let manifest = with_images(images);
        let env = envelope_json(TAG, &manifest);
        verify_release(&[key("k1", 1)], TAG, env.as_bytes(), &sig_line("k1", 1, &env), manifest.as_bytes())
    }

    #[test]
    fn a_release_may_list_the_images_it_built() {
        let m = verify_with_images(&format!(r#"[{{"name":"agent-base","ref":"{BASE}","digest":"{D1}"}}]"#)).unwrap();
        assert_eq!(m.image("agent-base").map(|i| (i.reference.as_str(), i.digest.as_str())), Some((BASE, D1)));
        assert_eq!(m.image("other"), None);
        // Releases without the field, and with it empty, have none.
        assert!(release_manifest().images.is_empty());
        assert!(verify_with_images("[]").unwrap().images.is_empty());
        // A registry on a port is a repository too.
        assert!(verify_with_images(&format!(r#"[{{"name":"b","ref":"localhost:5000/team/base","digest":"{D1}"}}]"#)).is_ok());
    }

    fn release_manifest() -> Manifest {
        let (env, man) = release();
        verify_release(&[key("k1", 1)], TAG, env.as_bytes(), &sig_line("k1", 1, &env), man.as_bytes()).unwrap()
    }

    #[test]
    fn an_image_that_breaks_the_format_is_refused_even_when_signed() {
        let bad = |name: &str, reference: &str, digest: &str| {
            let r = verify_with_images(&format!(r#"[{{"name":"{name}","ref":"{reference}","digest":"{digest}"}}]"#));
            assert!(matches!(r, Err(VerifyError::Manifest(_))), "{name} {reference} {digest}: {r:?}");
        };
        for digest in ["", "sha256:abc", "md5:1111111111111111111111111111111111111111111111111111111111111111", &D1.replace("sha256:", ""), &format!("{D1}0"), &D1.replace('1', "A")] {
            bad("agent-base", BASE, digest);
        }
        for name in ["", "Agent-Base", "agent base", "../x", "-x", &"a".repeat(65)] {
            bad(name, BASE, D1);
        }
        for reference in ["", "agent-base", "https://ghcr.io/x/y", "ghcr.io/x/y:1.0", "ghcr.io/x/y@sha256:abc", "ghcr.io//y", "ghcr.io/X/y", "ghcr.io/x/y/", "ghcr.io:/x/y", "ghcr.io:99999999/x/y", "ghcr.io/../y"] {
            bad("agent-base", reference, D1);
        }
        let twice = format!(r#"[{{"name":"agent-base","ref":"{BASE}","digest":"{D1}"}},{{"name":"agent-base","ref":"{BASE}","digest":"{D1}"}}]"#);
        assert!(matches!(verify_with_images(&twice), Err(VerifyError::Manifest(_))), "twice");
        let many: Vec<String> = (0..17).map(|i| format!(r#"{{"name":"i{i}","ref":"{BASE}","digest":"{D1}"}}"#)).collect();
        assert!(matches!(verify_with_images(&format!("[{}]", many.join(","))), Err(VerifyError::Manifest(_))), "too many");
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

    // A release that lists an image, as `scripts/release-manifest.mjs build --image` makes it and
    // signs it with the same seed-7 test key, copied byte for byte.
    #[test]
    fn a_release_with_an_image_made_by_the_script_verifies() {
        const PUBLIC_KEY_B64: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
        const SIGNATURES: &str = "k1 VQMuETqO/U3nRMhmcdyeH7AWC+QHUDbC8fRLumEwsmfzvQepLBcKX9J75BFw4V9p1/EowJDUvkHpZjQTTSpXCw==
";
        const ENVELOPE: &str = "{
  \"format\": 1,
  \"tag\": \"v0.60.1\",
  \"manifest_sha256\": \"56cea05f9aa7276f353e91c7748af2ad7a6631873d07a2a7d637e6f646883449\",
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
      \"size\": 5,
      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"
    },
    {
      \"name\": \"agentmux-0.60.1-x64-portable.7z\",
      \"size\": 8,
      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"
    }
  ],
  \"images\": [
    {
      \"name\": \"agent-base\",
      \"ref\": \"ghcr.io/agentmuxai/agent-base\",
      \"digest\": \"sha256:abababababababababababababababababababababababababababababababab\"
    }
  ]
}
";
        let public: [u8; 32] = BASE64.decode(PUBLIC_KEY_B64).unwrap().try_into().unwrap();
        let keys = [UpdateKey { id: "k1", public }];
        let m = verify_release(&keys, "v0.60.1", ENVELOPE.as_bytes(), SIGNATURES, MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.image("agent-base").map(|i| (i.reference.as_str(), i.digest.as_str())), Some((BASE, &*format!("sha256:{}", "ab".repeat(32)))));
        // The digest is covered: one byte changed after signing is refused.
        let swapped = MANIFEST.replace("abab", "abac");
        assert_eq!(verify_release(&keys, "v0.60.1", ENVELOPE.as_bytes(), SIGNATURES, swapped.as_bytes()), Err(VerifyError::ManifestMismatch));
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

    #[test]
    fn the_files_are_named_as_the_release_script_publishes_them() {
        let (e, s, m) = file_names("0.60.1");
        assert_eq!((e.as_str(), s.as_str(), m.as_str()), ("AgentMux-0.60.1-envelope.json", "AgentMux-0.60.1-envelope.json.sig", "AgentMux-0.60.1-manifest.json"));
    }

    #[test]
    fn a_verified_release_is_downloaded() {
        assert_eq!(decide("0.60.4", &Outcome::Verified), Ok(()));
    }

    #[test]
    fn an_unsigned_release_before_the_first_signed_one_is_downloaded_as_before() {
        assert_eq!(decide("0.59.40", &Outcome::Unsigned), Ok(()));
        assert_eq!(decide("v0.60.3", &Outcome::Unsigned), Ok(()));
    }

    #[test]
    fn an_unsigned_release_from_the_first_signed_one_on_is_refused() {
        for v in ["0.60.4", "0.60.5", "0.61.0", "1.0.0"] {
            let e = decide(v, &Outcome::Unsigned).unwrap_err();
            assert!(e.contains("wasn't signed") && e.contains("release page"), "{v}: {e}");
        }
        // A version that can't be read is not trusted to be an old one.
        assert!(decide("nightly", &Outcome::Unsigned).is_err());
    }

    #[test]
    fn a_bad_signature_or_a_disagreement_is_refused_with_the_signature_message() {
        for o in [Outcome::Failed("x".into()), Outcome::Disagrees("y".into())] {
            let e = decide("0.60.4", &o).unwrap_err();
            assert!(e.contains("The signature on v0.60.4 doesn't match") && e.contains("tell the AgentMux team"), "{e}");
        }
    }

    #[test]
    fn a_release_without_a_file_for_this_install_and_an_unreachable_check_are_refused_plainly() {
        assert_eq!(decide("0.60.4", &Outcome::NotListed("z".into())).unwrap_err(), "v0.60.4 has no file for this kind of install.");
        let e = decide("0.60.4", &Outcome::Unreachable("GitHub didn't answer".into())).unwrap_err();
        assert!(e.contains("Couldn't check the signature of v0.60.4") && e.contains("GitHub didn't answer") && e.contains("Try again"), "{e}");
    }

    #[test]
    fn the_first_signed_release_is_the_one_the_pipeline_first_signed() {
        assert_eq!(FIRST_SIGNED, (0, 60, 4));
    }

    #[test]
    fn a_version_is_read_down_to_its_three_numbers() {
        assert_eq!(version_core("0.59.100"), Some((0, 59, 100)));
        assert_eq!(version_core("v1.2.3+g1a2b"), Some((1, 2, 3)));
        assert_eq!(version_core("1.2"), None);
        assert_eq!(version_core("1.2.3.4"), None);
    }

    #[test]
    fn a_release_is_judged_by_the_name_size_and_hash_of_the_file_it_signs() {
        let k = key("k1", 7);
        let hash = "8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73";
        let manifest = manifest_json(&[("AgentMux_0.60.1_amd64.deb", 100, hash)]);
        let envelope = envelope_json(TAG, &manifest);
        let sig = sig_line("k1", 7, &envelope);
        let judged = |name: &str, size: u64, sha: &str| judge_file(&[k], TAG, Some((envelope.as_bytes(), sig.as_str(), manifest.as_bytes())), name, size, sha);
        assert_eq!(judged("AgentMux_0.60.1_amd64.deb", 100, hash), Outcome::Verified);
        assert!(matches!(judged("AgentMux_0.60.1_amd64.deb", 101, hash), Outcome::Disagrees(_)));
        assert!(matches!(judged("AgentMux_0.60.1_amd64.deb", 100, &"0".repeat(64)), Outcome::Disagrees(_)));
        assert!(matches!(judged("AgentMux-0.60.1-1.x86_64.rpm", 100, hash), Outcome::NotListed(_)));
        assert_eq!(judge_file(&[k], TAG, None, "x", 1, hash), Outcome::Unsigned);
    }

    // A release that carries an `upgrade` object, as `scripts/release-manifest.mjs build` makes it (the floor from
    // UPGRADE_FLOOR, and `--shared-store-changed true`), signed with the same seed-7 test key, copied byte for byte.
    const UPGRADE_PUBLIC_KEY_B64: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
    const UPGRADE_SIGNATURES: &str = "k1 wf8ru0NTcp3G7GCZff/1dX9O2SEd7nSHuacY058sSrcQHFkpidkkwklrjqb1hJOr61diLh/2rRGg32Gnn5whDw==
";
    const UPGRADE_ENVELOPE: &str = "{
  \"format\": 1,
  \"tag\": \"v0.60.1\",
  \"manifest_sha256\": \"2038d37391a59d7b22bb0bee89eb31b6f94df10d4730389b79808fb32a52272f\",
  \"created_at\": \"2026-10-25T12:00:00Z\"
}
";
    const UPGRADE_MANIFEST: &str = "{
  \"format\": 1,
  \"version\": \"0.60.1\",
  \"tag\": \"v0.60.1\",
  \"files\": [
    {
      \"name\": \"AgentMux-0.60.1-x64-setup.exe\",
      \"size\": 5,
      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"
    },
    {
      \"name\": \"agentmux-0.60.1-x64-portable.7z\",
      \"size\": 8,
      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"
    }
  ],
  \"upgrade\": {
    \"floor\": \"0.55.0\",
    \"shared_store_changed\": true
  }
}
";

    fn upgrade_keys() -> [UpdateKey; 1] {
        let public: [u8; 32] = BASE64.decode(UPGRADE_PUBLIC_KEY_B64).unwrap().try_into().unwrap();
        [UpdateKey { id: "k1", public }]
    }

    #[test]
    fn a_release_with_an_upgrade_object_made_by_the_script_verifies_and_gives_its_floor() {
        let m = verify_release(&upgrade_keys(), "v0.60.1", UPGRADE_ENVELOPE.as_bytes(), UPGRADE_SIGNATURES, UPGRADE_MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.signed_floor(), SignedFloor::Valid("0.55.0".into()));
        assert_eq!(m.shared_store_changed(), SharedStoreChanged::Known(true));
    }

    #[test]
    fn a_floor_changed_after_signing_is_refused() {
        // The floor is covered: the manifest's hash is in the signed envelope, so one changed byte fails verification.
        let lowered = UPGRADE_MANIFEST.replace("\"floor\": \"0.55.0\"", "\"floor\": \"0.40.0\"");
        assert_ne!(lowered, UPGRADE_MANIFEST);
        assert_eq!(verify_release(&upgrade_keys(), "v0.60.1", UPGRADE_ENVELOPE.as_bytes(), UPGRADE_SIGNATURES, lowered.as_bytes()), Err(VerifyError::ManifestMismatch));
        let stripped = UPGRADE_MANIFEST.replace("\"shared_store_changed\": true", "\"shared_store_changed\": false");
        assert_eq!(verify_release(&upgrade_keys(), "v0.60.1", UPGRADE_ENVELOPE.as_bytes(), UPGRADE_SIGNATURES, stripped.as_bytes()), Err(VerifyError::ManifestMismatch));
    }

    #[test]
    fn an_app_that_does_not_know_the_field_reads_the_same_release() {
        // The manifest and envelope shapes as they were before `upgrade` existed, frozen here: they ignore what they do not
        // know, so an installed app keeps verifying a release that carries it.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct OldManifest {
            format: u32,
            version: String,
            tag: String,
            files: Vec<ManifestFile>,
            #[serde(default)]
            images: Vec<ManifestImage>,
        }
        let old: OldManifest = serde_json::from_str(UPGRADE_MANIFEST).unwrap();
        assert_eq!((old.format, old.version.as_str(), old.tag.as_str(), old.files.len(), old.images.len()), (1, "0.60.1", "v0.60.1", 2, 0));
        let _: Envelope = serde_json::from_str(UPGRADE_ENVELOPE).unwrap();
    }

    fn manifest_with_upgrade(upgrade: &str) -> Manifest {
        let text = format!(r#"{{"format":1,"version":"0.60.1","tag":"v0.60.1","files":[{{"name":"a.exe","size":1,"sha256":"{}"}}]{upgrade}}}"#, "0".repeat(64));
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn the_signed_floor_is_valid_only_as_a_plain_version_not_above_the_release() {
        let floor = |u: &str| manifest_with_upgrade(&format!(r#","upgrade":{u}"#)).signed_floor();
        assert_eq!(manifest_with_upgrade("").signed_floor(), SignedFloor::Absent);
        assert_eq!(floor(r#"{"floor":"0.55.0"}"#), SignedFloor::Valid("0.55.0".into()));
        // At the release's own version is allowed; one above it is not.
        assert_eq!(floor(r#"{"floor":"0.60.1"}"#), SignedFloor::Valid("0.60.1".into()));
        assert!(matches!(floor(r#"{"floor":"0.60.2"}"#), SignedFloor::Unusable(why) if why.contains("above the release")));
        for bad in [r#"{"floor":"soon"}"#, r#"{"floor":"0.55"}"#, r#"{"floor":"v0.55.0"}"#, r#"{"floor":"0.55.0-rc1"}"#, r#"{"floor":""}"#, r#"{"floor":55}"#, r#"{"floor":null}"#, r#"{"floor":["0.55.0"]}"#, r#""0.55.0""#, "5", "[]", "true"] {
            assert!(matches!(floor(bad), SignedFloor::Unusable(_)), "{bad}");
        }
        // An object with no floor says nothing about it; a field this build does not know is ignored.
        assert_eq!(floor(r#"{"shared_store_changed":true}"#), SignedFloor::Absent);
        assert_eq!(floor(r#"{"floor":"0.55.0","future":{"a":1}}"#), SignedFloor::Valid("0.55.0".into()));
    }

    #[test]
    fn shared_store_changed_is_true_false_or_absent_and_never_touches_the_floor() {
        let read = |u: &str| manifest_with_upgrade(&format!(r#","upgrade":{u}"#));
        assert_eq!(manifest_with_upgrade("").shared_store_changed(), SharedStoreChanged::Absent);
        assert_eq!(read(r#"{"floor":"0.55.0"}"#).shared_store_changed(), SharedStoreChanged::Absent);
        assert_eq!(read(r#"{"floor":"0.55.0","shared_store_changed":true}"#).shared_store_changed(), SharedStoreChanged::Known(true));
        assert_eq!(read(r#"{"floor":"0.55.0","shared_store_changed":false}"#).shared_store_changed(), SharedStoreChanged::Known(false));
        for bad in [r#""yes""#, "1", "null", "[]"] {
            let m = read(&format!(r#"{{"floor":"0.55.0","shared_store_changed":{bad}}}"#));
            assert_eq!(m.shared_store_changed(), SharedStoreChanged::Unusable, "{bad}");
            // A bad flag never changes the floor.
            assert_eq!(m.signed_floor(), SignedFloor::Valid("0.55.0".into()), "{bad}");
        }
        // And a bad floor never changes the flag.
        let m = read(r#"{"floor":"soon","shared_store_changed":true}"#);
        assert_eq!(m.shared_store_changed(), SharedStoreChanged::Known(true));
        assert!(matches!(m.signed_floor(), SignedFloor::Unusable(_)));
    }

    #[test]
    fn a_bad_floor_does_not_fail_the_verification_of_the_files() {
        let manifest = UPGRADE_MANIFEST.replace("\"floor\": \"0.55.0\"", "\"floor\": \"soon\"");
        let envelope = UPGRADE_ENVELOPE.replace("2038d37391a59d7b22bb0bee89eb31b6f94df10d4730389b79808fb32a52272f", &hex(&Sha256::digest(manifest.as_bytes())));
        // Signed by the test key (seed 7) the way a pipeline bug could have signed it.
        let sig_line = format!("k1 {}", BASE64.encode(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]).sign(envelope.as_bytes()).to_bytes()));
        let m = verify_release(&upgrade_keys(), "v0.60.1", envelope.as_bytes(), &sig_line, manifest.as_bytes()).unwrap();
        assert!(matches!(m.signed_floor(), SignedFloor::Unusable(_)));
        assert_eq!(m.file("AgentMux-0.60.1-x64-setup.exe").map(|f| f.size), Some(5));
    }

    #[test]
    fn a_plain_version_is_three_numbers_and_nothing_else() {
        for ok in ["0.55.0", "1.2.3", "10.20.300"] {
            assert!(is_plain_version(ok), "{ok}");
        }
        for bad in ["", "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.3-rc1", "1.2.x", " 1.2.3", "1.2.3\n", "1..3", "1234567890.1.1"] {
            assert!(!is_plain_version(bad), "{bad:?}");
        }
    }
}
