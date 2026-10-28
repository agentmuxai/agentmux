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

pub(crate) use agentmux_common::update_manifest::{decide, file_names, Outcome};
use agentmux_common::update_manifest::{judge_file, UpdateKey, MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS};

use crate::backend::update_check::ReleaseAsset;
use crate::backend::update_download::RELEASE_DOWNLOAD_PREFIX;

/// The decision, given what was fetched (`None`: the release has no signed
/// manifest) and the file about to be downloaded.
pub(crate) fn judge(keys: &[UpdateKey], tag: &str, fetched: Option<(&[u8], &str, &[u8])>, asset: &ReleaseAsset) -> Outcome {
    judge_file(keys, tag, fetched, &asset.name, asset.size, &asset.sha256)
}

/// Why a small release file wasn't read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FetchError {
    /// GitHub didn't answer, or answered with an error status.
    Transport(String),
    /// It answered with more than the file may hold.
    TooLarge(usize),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transport(why) => f.write_str(why),
            FetchError::TooLarge(cap) => write!(f, "larger than {cap} bytes"),
        }
    }
}

/// Fetch one small release file: `None` when the release has no such file (404),
/// an error for anything else, and never more than `cap` bytes.
pub(crate) async fn fetch_small(client: &reqwest::Client, url: &str, cap: usize) -> Result<Option<Vec<u8>>, FetchError> {
    let transport = |why: String| FetchError::Transport(why);
    let mut resp = tokio::time::timeout(std::time::Duration::from_secs(20), client.get(url).send())
        .await
        .map_err(|_| transport("GitHub didn't answer".to_string()))?
        .map_err(|e| transport(e.to_string()))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(transport(format!("HTTP {}", resp.status())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = tokio::time::timeout(std::time::Duration::from_secs(20), resp.chunk())
        .await
        .map_err(|_| transport("GitHub stopped answering".to_string()))?
        .map_err(|e| transport(e.to_string()))?
    {
        if body.len() + chunk.len() > cap {
            return Err(FetchError::TooLarge(cap));
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

    #[tokio::test]
    async fn an_address_off_the_releases_is_not_fetched() {
        let mut a = asset("AgentMux-0.60.1-x64-setup.exe", 100, SETUP_SHA);
        a.url = "https://example.com/AgentMux-0.60.1-x64-setup.exe".into();
        assert!(matches!(check("0.60.1", &a).await, Outcome::Failed(w) if w.contains("isn't on AgentMux's GitHub releases")));
        // ...and that is a refusal for a signed-era release, never a download.
        assert!(gate("0.60.4", &a).await.is_err());
    }
}
