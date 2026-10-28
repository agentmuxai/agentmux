// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Signed update checksums (`docs/specs/SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md` §5, §7, §8).
//! When a download starts, this fetches the release's signed manifest (envelope,
//! signature, manifest: a few KB), checks it against the key this build pins
//! (`agentmux_common::update_manifest`) and compares it with the file about to be
//! downloaded. Step 3, **enforce**: a release from the first signed one on is
//! downloaded only when its manifest verifies and agrees; one that doesn't verify,
//! disagrees, lacks the file or isn't signed is refused with the message the person
//! reads (§7). A release made before signing existed (`FIRST_SIGNED`) still reads as
//! `unsigned` and is downloaded as before, so nobody on an old release is stranded.

use agentmux_common::update_manifest::{verify_release, Manifest, UpdateKey, MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS};

use crate::backend::update_check::version_core;

use crate::backend::update_check::ReleaseAsset;
use crate::backend::update_download::RELEASE_DOWNLOAD_PREFIX;

/// What the signed manifest said about the file about to be downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The release publishes no signed manifest (made before signing).
    Unsigned,
    /// Signed and valid; the file is listed and matches the digest the download uses.
    Verified,
    /// Signed and valid, but the file isn't listed in it.
    NotListed(String),
    /// Signed and valid, but the file's size or hash differ from the digest the download uses.
    Disagrees(String),
    /// A manifest is there but doesn't verify (or isn't what the envelope signed).
    Failed(String),
    /// The signed files couldn't be fetched (GitHub didn't answer, an error status): nothing is known.
    Unreachable(String),
}

/// The first release the pipeline signs (v0.60.4). From it on, a release without a
/// verified signed manifest is not downloaded; an older one never had one.
pub(crate) const FIRST_SIGNED: (u64, u64, u64) = (0, 60, 4);

/// The three files of a signed release, as published next to its other files.
pub(crate) fn file_names(version: &str) -> (String, String, String) {
    (
        format!("AgentMux-{version}-envelope.json"),
        format!("AgentMux-{version}-envelope.json.sig"),
        format!("AgentMux-{version}-manifest.json"),
    )
}

/// The decision, given what was fetched (`None`: the release has no signed
/// manifest) and the file about to be downloaded.
pub(crate) fn judge(keys: &[UpdateKey], tag: &str, fetched: Option<(&[u8], &str, &[u8])>, asset: &ReleaseAsset) -> Outcome {
    let Some((envelope, signatures, manifest)) = fetched else {
        return Outcome::Unsigned;
    };
    let manifest: Manifest = match verify_release(keys, tag, envelope, signatures, manifest) {
        Ok(m) => m,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    match manifest.file(&asset.name) {
        None => Outcome::NotListed(format!("{} isn't in the signed manifest", asset.name)),
        Some(f) if f.size != asset.size => Outcome::Disagrees(format!("size {} in the manifest, {} in the download's listing", f.size, asset.size)),
        Some(f) if !f.sha256.eq_ignore_ascii_case(&asset.sha256) => {
            Outcome::Disagrees("the manifest's SHA-256 differs from the digest the download uses".into())
        }
        Some(_) => Outcome::Verified,
    }
}

/// Fetch one small release file: `None` when the release has no such file (404),
/// an error for anything else, and never more than `cap` bytes.
async fn fetch_small(client: &reqwest::Client, url: &str, cap: usize) -> Result<Option<Vec<u8>>, String> {
    let mut resp = tokio::time::timeout(std::time::Duration::from_secs(20), client.get(url).send())
        .await
        .map_err(|_| "GitHub didn't answer".to_string())?
        .map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = tokio::time::timeout(std::time::Duration::from_secs(20), resp.chunk())
        .await
        .map_err(|_| "GitHub stopped answering".to_string())?
        .map_err(|e| e.to_string())?
    {
        if body.len() + chunk.len() > cap {
            return Err(format!("larger than {cap} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// Fetch the signed manifest of `asset`'s release and judge it. Release files
/// sit side by side, so the others' addresses are the asset's with the name
/// swapped; an address that isn't on AgentMux's releases is not fetched.
pub(crate) async fn check(version: &str, asset: &ReleaseAsset) -> Outcome {
    let tag = format!("v{version}");
    let Some(dir) = asset.url.strip_suffix(asset.name.as_str()).filter(|d| d.starts_with(RELEASE_DOWNLOAD_PREFIX)) else {
        return Outcome::Failed("the release file isn't on AgentMux's GitHub releases".into());
    };
    let client = match reqwest::Client::builder()
        .user_agent(format!("AgentMux/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Unreachable(e.to_string()),
    };
    let (envelope_name, signature_name, manifest_name) = file_names(version);
    let envelope = match fetch_small(&client, &format!("{dir}{envelope_name}"), MAX_ENVELOPE_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => return Outcome::Unsigned,
        Err(e) => return Outcome::Unreachable(format!("couldn't fetch the envelope: {e}")),
    };
    let signatures = match fetch_small(&client, &format!("{dir}{signature_name}"), MAX_ENVELOPE_BYTES).await {
        Ok(Some(b)) => String::from_utf8_lossy(&b).into_owned(),
        Ok(None) => return Outcome::Failed("the envelope has no signature file".into()),
        Err(e) => return Outcome::Unreachable(format!("couldn't fetch the signature: {e}")),
    };
    let manifest = match fetch_small(&client, &format!("{dir}{manifest_name}"), MAX_MANIFEST_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => return Outcome::Failed("the envelope has no manifest".into()),
        Err(e) => return Outcome::Unreachable(format!("couldn't fetch the manifest: {e}")),
    };
    judge(UPDATE_KEYS, &tag, Some((&envelope, &signatures, &manifest)), asset)
}

/// What to do about `outcome` for `version`: `Ok` to download, or the message to show (§7).
pub(crate) fn decide(version: &str, outcome: &Outcome) -> Result<(), String> {
    let v = format!("v{}", version.trim_start_matches('v'));
    let signature_wrong = format!("The signature on {v} doesn't match, so AgentMux won't download it. Don't install it; tell the AgentMux team.");
    match outcome {
        Outcome::Verified => Ok(()),
        Outcome::Unsigned => match version_core(version) {
            // Made before signing existed: downloaded as before.
            Some(core) if core < FIRST_SIGNED => Ok(()),
            _ => Err(format!("{v} wasn't signed, so AgentMux won't download it itself. Get it from the release page.")),
        },
        Outcome::NotListed(_) => Err(format!("{v} has no file for this kind of install.")),
        Outcome::Disagrees(_) | Outcome::Failed(_) => Err(signature_wrong),
        Outcome::Unreachable(why) => Err(format!("Couldn't check the signature of {v} ({why}). Try again.")),
    }
}

/// Check the release's signed manifest, log what was found, and say whether the download may start.
pub(crate) async fn gate(version: &str, asset: &ReleaseAsset) -> Result<(), String> {
    let outcome = check(version, asset).await;
    let file = &asset.name;
    match &outcome {
        Outcome::Unsigned => tracing::info!(%version, %file, "update signature: the release has no signed manifest"),
        Outcome::Verified => tracing::info!(%version, %file, "update signature: verified; the signed manifest matches the digest in use"),
        Outcome::NotListed(why) | Outcome::Disagrees(why) => tracing::error!(%version, %file, %why, "update signature: verified, but it disagrees with the file offered"),
        Outcome::Failed(why) => tracing::error!(%version, %file, %why, "update signature: the signed manifest did not verify"),
        Outcome::Unreachable(why) => tracing::warn!(%version, %file, %why, "update signature: couldn't fetch the signed manifest"),
    }
    decide(version, &outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same fixed release the common crate and the release script test (seed-7 key).
    const KEY: [u8; 32] = [
        0xea, 0x4a, 0x6c, 0x63, 0xe2, 0x9c, 0x52, 0x0a, 0xbe, 0xf5, 0x50, 0x7b, 0x13, 0x2e, 0xc5, 0xf9, 0x95, 0x47, 0x76, 0xae, 0xbe, 0xbe, 0x7b, 0x92, 0x42,
        0x1e, 0xea, 0x69, 0x14, 0x46, 0xd2, 0x2c,
    ];
    const MANIFEST: &str = "{\n  \"format\": 1,\n  \"version\": \"0.60.1\",\n  \"tag\": \"v0.60.1\",\n  \"files\": [\n    {\n      \"name\": \"AgentMux-0.60.1-x64-setup.exe\",\n      \"size\": 100,\n      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"\n    },\n    {\n      \"name\": \"agentmux-0.60.1-x64-portable.7z\",\n      \"size\": 50,\n      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"\n    }\n  ]\n}\n";
    const ENVELOPE: &str = "{\n  \"format\": 1,\n  \"tag\": \"v0.60.1\",\n  \"manifest_sha256\": \"8a83f0bb7ce99e7537b872b55d6e688da6713ebd078b0dd88c740b1a3c5809eb\",\n  \"created_at\": \"2026-10-25T12:00:00Z\"\n}\n";
    const SIG: &str = "k1 oGZeVKdahQJP1mqd964PNT7Fie6IbSoUMkDXrmi1QbUVn947HuHtRZNRJt36L22rcAkLPGRWolco8DB2AZXZCw==\n";
    const SETUP_SHA: &str = "8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73";

    fn keys() -> Vec<UpdateKey> {
        vec![UpdateKey { id: "k1", public: KEY }]
    }

    fn asset(name: &str, size: u64, sha256: &str) -> ReleaseAsset {
        ReleaseAsset { name: name.into(), size, url: format!("{RELEASE_DOWNLOAD_PREFIX}v0.60.1/{name}"), sha256: sha256.into() }
    }

    fn judged(asset: &ReleaseAsset) -> Outcome {
        judge(&keys(), "v0.60.1", Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())), asset)
    }

    #[test]
    fn a_release_without_a_signed_manifest_reads_as_unsigned() {
        assert_eq!(judge(&keys(), "v0.60.1", None, &asset("AgentMux-0.60.1-x64-setup.exe", 100, SETUP_SHA)), Outcome::Unsigned);
    }

    #[test]
    fn the_signed_manifest_agreeing_with_the_digest_in_use_is_verified() {
        assert_eq!(judged(&asset("AgentMux-0.60.1-x64-setup.exe", 100, SETUP_SHA)), Outcome::Verified);
        // The listing's digest is case-insensitive hex.
        assert_eq!(judged(&asset("AgentMux-0.60.1-x64-setup.exe", 100, &SETUP_SHA.to_uppercase())), Outcome::Verified);
    }

    #[test]
    fn a_file_the_manifest_does_not_describe_that_way_is_a_disagreement_not_a_failure() {
        assert!(matches!(judged(&asset("AgentMux-0.60.1-x64-setup.exe", 101, SETUP_SHA)), Outcome::Disagrees(w) if w.contains("size")));
        assert!(matches!(judged(&asset("AgentMux-0.60.1-x64-setup.exe", 100, &"0".repeat(64))), Outcome::Disagrees(w) if w.contains("SHA-256")));
        assert!(matches!(judged(&asset("AgentMux-0.60.1-other.exe", 100, SETUP_SHA)), Outcome::NotListed(w) if w.contains("isn't in the signed manifest")));
    }

    #[test]
    fn a_manifest_that_does_not_verify_is_a_failure_with_the_reason() {
        let a = asset("AgentMux-0.60.1-x64-setup.exe", 100, SETUP_SHA);
        // Another key, a different release's tag, a tampered manifest, a missing signature line.
        let other = vec![UpdateKey { id: "k1", public: [9; 32] }];
        assert!(matches!(judge(&other, "v0.60.1", Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())), &a), Outcome::Failed(_)));
        assert!(matches!(judge(&keys(), "v0.60.2", Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())), &a), Outcome::Failed(w) if w.contains("v0.60.2")));
        let tampered = MANIFEST.replace("\"size\": 100", "\"size\": 101");
        assert!(matches!(judge(&keys(), "v0.60.1", Some((ENVELOPE.as_bytes(), SIG, tampered.as_bytes())), &a), Outcome::Failed(_)));
        assert!(matches!(judge(&keys(), "v0.60.1", Some((ENVELOPE.as_bytes(), "", MANIFEST.as_bytes())), &a), Outcome::Failed(_)));
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

    #[tokio::test]
    async fn an_address_off_the_releases_is_not_fetched() {
        let mut a = asset("AgentMux-0.60.1-x64-setup.exe", 100, SETUP_SHA);
        a.url = "https://example.com/AgentMux-0.60.1-x64-setup.exe".into();
        assert!(matches!(check("0.60.1", &a).await, Outcome::Failed(w) if w.contains("isn't on AgentMux's GitHub releases")));
        // ...and that is a refusal for a signed-era release, never a download.
        assert!(gate("0.60.4", &a).await.is_err());
    }
}
