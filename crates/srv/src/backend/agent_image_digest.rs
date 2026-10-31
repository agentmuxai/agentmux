// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The digest of the default agent image, from the signed manifest of the release this srv is.
//!
//! The image tag `agent-base:<version>` can be pushed by anything that can write the package, so
//! a tag alone doesn't say which image the release built. The release's signed manifest does:
//! it lists the image's digest (`agentmux_common::update_manifest::ManifestImage`), covered by
//! the same signature as the release's files. A container is created from the image at that
//! digest, tagged locally as `agent-base:<version>` (`container::pull::pinned`).
//!
//! Fetched when a container is created, kept for the life of srv, and never fetched on a turn
//! of a container that exists. What it does when it can't tell:
//! - a verified manifest that lists the image: [`Pin::Digest`];
//! - no manifest (a build that was never released, a release made before signing), none that
//!   can be reached, or one that lists no image: [`Pin::Unpinned`], and the version tag is
//!   pulled as before, with a log line saying the image is unverified;
//! - a manifest that is there but does not verify: [`Pin::Refused`], and the default image is
//!   not pulled.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agentmux_common::update_manifest::{verify_release, UpdateKey, MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS};

use crate::backend::container_image::AGENT_BASE_REPO;
use crate::backend::update_download::RELEASE_DOWNLOAD_PREFIX;
use crate::backend::update_signature::{fetch_small, file_names, FetchError};

/// The name the release lists the default agent image under.
pub(crate) const IMAGE_NAME: &str = "agent-base";

/// The name it lists the developer variant under (the same repository, another image).
pub(crate) const DEVELOPER_IMAGE_NAME: &str = "agent-base-developer";

/// The version of the release this srv is.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How long all three manifest files may take to fetch: a container is waiting for it.
const FETCH_TOTAL: Duration = Duration::from_secs(12);

/// How long an answer of "can't tell" is kept before the next container asks again.
const UNPINNED_KEPT: Duration = Duration::from_secs(120);

/// What to run the default agent image from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pin {
    /// The release's own image: `ghcr.io/agentmuxai/agent-base@sha256:<digest>`.
    Digest(String),
    /// Nothing signed says which image; why, for the log.
    Unpinned(String),
    /// A manifest is there and is not trusted; why, for the person.
    Refused(String),
}

/// What was fetched of a release's signed manifest.
pub(crate) enum Fetched {
    /// The release has no envelope (made before signing, or never released).
    NoManifest,
    /// GitHub could not be reached, or answered with something unusable.
    Unreachable(String),
    /// The envelope is there but a file it needs isn't.
    Incomplete(String),
    Files { envelope: Vec<u8>, signatures: String, manifest: Vec<u8> },
}

/// The decision for the image the release lists as `image_name`, given what was fetched for the release called `version`.
pub(crate) fn decide(keys: &[UpdateKey], version: &str, image_name: &str, fetched: Fetched) -> Pin {
    let (envelope, signatures, manifest) = match fetched {
        Fetched::NoManifest => return Pin::Unpinned("the release has no signed manifest".into()),
        Fetched::Unreachable(why) => return Pin::Unpinned(format!("the signed manifest couldn't be fetched: {why}")),
        Fetched::Incomplete(why) => return Pin::Refused(why),
        Fetched::Files { envelope, signatures, manifest } => (envelope, signatures, manifest),
    };
    let manifest = match verify_release(keys, &format!("v{version}"), &envelope, &signatures, &manifest) {
        Ok(m) => m,
        Err(e) => return Pin::Refused(e.to_string()),
    };
    match manifest.image(image_name) {
        None => Pin::Unpinned(format!("the signed manifest lists no {image_name} image")),
        // The repository is this app's own, never one the manifest names.
        Some(image) if image.reference != AGENT_BASE_REPO => Pin::Unpinned(format!("the signed manifest lists the agent image under {}", image.reference)),
        Some(image) => Pin::Digest(format!("{AGENT_BASE_REPO}@{}", image.digest)),
    }
}

/// A file that couldn't be read: GitHub not answering is "can't tell", but a file larger than any
/// such file may be is the release's metadata being wrong, which is refused like any other.
fn unread(error: FetchError, file: &str) -> Fetched {
    match error {
        FetchError::Transport(why) => Fetched::Unreachable(why),
        FetchError::TooLarge(cap) => Fetched::Incomplete(format!("the release's {file} is larger than {cap} bytes")),
    }
}

/// Fetch the three files of the signed release called `version`.
async fn fetch(version: &str) -> Fetched {
    let client = match reqwest::Client::builder()
        .user_agent(format!("AgentMux/{CURRENT}"))
        .connect_timeout(Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Fetched::Unreachable(e.to_string()),
    };
    let dir = format!("{RELEASE_DOWNLOAD_PREFIX}v{version}/");
    let (envelope_name, signature_name, manifest_name) = file_names(version);
    let envelope = match fetch_small(&client, &format!("{dir}{envelope_name}"), MAX_ENVELOPE_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => return Fetched::NoManifest,
        Err(e) => return unread(e, "envelope"),
    };
    let signatures = match fetch_small(&client, &format!("{dir}{signature_name}"), MAX_ENVELOPE_BYTES).await {
        Ok(Some(b)) => String::from_utf8_lossy(&b).into_owned(),
        Ok(None) => return Fetched::Incomplete("the release's envelope has no signature".into()),
        Err(e) => return unread(e, "signature"),
    };
    let manifest = match fetch_small(&client, &format!("{dir}{manifest_name}"), MAX_MANIFEST_BYTES).await {
        Ok(Some(b)) => b,
        Ok(None) => return Fetched::Incomplete("the release's envelope has no manifest".into()),
        Err(e) => return unread(e, "manifest"),
    };
    Fetched::Files { envelope, signatures, manifest }
}

/// What to run the default agent image of this srv's own version from. Fetches once; a verified
/// answer (a digest, or a refusal) stands for the life of srv, "can't tell" for a couple of
/// minutes.
pub(crate) async fn pin_for_running_version(image_name: &str) -> Pin {
    static KEPT: OnceLock<tokio::sync::Mutex<std::collections::HashMap<String, (Instant, Pin)>>> = OnceLock::new();
    let mut kept = KEPT.get_or_init(Default::default).lock().await;
    if let Some((at, pin)) = kept.get(image_name) {
        if !matches!(pin, Pin::Unpinned(_)) || at.elapsed() < UNPINNED_KEPT {
            return pin.clone();
        }
    }
    let fetched = tokio::time::timeout(FETCH_TOTAL, fetch(CURRENT)).await.unwrap_or_else(|_| Fetched::Unreachable("GitHub didn't answer in time".into()));
    let pin = decide(UPDATE_KEYS, CURRENT, image_name, fetched);
    match &pin {
        Pin::Digest(reference) => tracing::info!(version = CURRENT, image = image_name, %reference, "the signed manifest names the agent image"),
        Pin::Unpinned(why) => tracing::info!(version = CURRENT, image = image_name, %why, "the agent image can't be checked against a signed manifest; using its version tag, unverified"),
        Pin::Refused(why) => tracing::warn!(version = CURRENT, image = image_name, %why, "the release's signed manifest isn't trusted; not pulling the agent image"),
    }
    kept.insert(image_name.to_string(), (Instant::now(), pin.clone()));
    pin
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    const VERSION: &str = "0.60.1";
    const H: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn key() -> UpdateKey {
        UpdateKey { id: "k1", public: SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes() }
    }

    fn release(images: &str) -> Fetched {
        let manifest = format!(r#"{{"format":1,"version":"{VERSION}","tag":"v{VERSION}","files":[{{"name":"a.exe","size":1,"sha256":"{H}"}}],"images":{images}}}"#);
        let sha: String = Sha256::digest(manifest.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let envelope = format!(r#"{{"format":1,"tag":"v{VERSION}","manifest_sha256":"{sha}","created_at":"2026-10-28T00:00:00Z"}}"#);
        let signatures = format!("k1 {}\n", BASE64.encode(SigningKey::from_bytes(&[3; 32]).sign(envelope.as_bytes()).to_bytes()));
        Fetched::Files { envelope: envelope.into_bytes(), signatures, manifest: manifest.into_bytes() }
    }

    fn image(reference: &str) -> String {
        format!(r#"[{{"name":"agent-base","ref":"{reference}","digest":"sha256:{H}"}}]"#)
    }

    #[test]
    fn a_verified_manifest_that_lists_the_image_pins_it_by_digest() {
        assert_eq!(decide(&[key()], VERSION, IMAGE_NAME, release(&image(AGENT_BASE_REPO))), Pin::Digest(format!("{AGENT_BASE_REPO}@sha256:{H}")));
    }

    #[test]
    fn the_developer_variant_is_pinned_by_its_own_listing() {
        let both = format!(
            r#"[{{"name":"agent-base","ref":"{AGENT_BASE_REPO}","digest":"sha256:{H}"}},{{"name":"agent-base-developer","ref":"{AGENT_BASE_REPO}","digest":"sha256:{}"}}]"#,
            "3".repeat(64)
        );
        assert_eq!(decide(&[key()], VERSION, DEVELOPER_IMAGE_NAME, release(&both)), Pin::Digest(format!("{AGENT_BASE_REPO}@sha256:{}", "3".repeat(64))));
        assert_eq!(decide(&[key()], VERSION, IMAGE_NAME, release(&both)), Pin::Digest(format!("{AGENT_BASE_REPO}@sha256:{H}")), "the base image keeps its own digest");
        let only_base = decide(&[key()], VERSION, DEVELOPER_IMAGE_NAME, release(&image(AGENT_BASE_REPO)));
        assert!(matches!(only_base, Pin::Unpinned(why) if why.contains("agent-base-developer")), "a release without one: its tag is used, unverified");
    }

    #[test]
    fn without_a_signed_listing_the_version_tag_is_used_unverified() {
        let unpinned = |f: Fetched| matches!(decide(&[key()], VERSION, IMAGE_NAME, f), Pin::Unpinned(_));
        assert!(unpinned(Fetched::NoManifest), "a build that was never released");
        assert!(unpinned(Fetched::Unreachable("offline".into())));
        assert!(unpinned(release("[]")), "a release before the field");
        assert!(unpinned(release(&image("ghcr.io/someone/else"))), "never a repository the app didn't choose");
    }

    #[test]
    fn a_file_over_its_cap_is_refused_and_a_silent_github_is_not() {
        let transport = unread(FetchError::Transport("HTTP 503".into()), "manifest");
        assert!(matches!(decide(&[key()], VERSION, IMAGE_NAME, transport), Pin::Unpinned(_)));
        let padded = unread(FetchError::TooLarge(4096), "envelope");
        assert!(matches!(decide(&[key()], VERSION, IMAGE_NAME, padded), Pin::Refused(why) if why.contains("envelope") && why.contains("4096")));
    }

    #[test]
    fn a_manifest_that_is_there_and_not_trusted_refuses_the_default_image() {
        let refused = |f: Fetched, keys: &[UpdateKey], version: &str| matches!(decide(keys, version, IMAGE_NAME, f), Pin::Refused(_));
        assert!(refused(release(&image(AGENT_BASE_REPO)), &[UpdateKey { id: "k1", public: [9; 32] }], VERSION), "another key");
        assert!(refused(release(&image(AGENT_BASE_REPO)), &[], VERSION), "no key pinned");
        assert!(refused(release(&image(AGENT_BASE_REPO)), &[key()], "0.60.2"), "another release's manifest");
        assert!(refused(Fetched::Incomplete("no signature".into()), &[key()], VERSION));
        let Fetched::Files { envelope, signatures, manifest } = release(&image(AGENT_BASE_REPO)) else { unreachable!() };
        let tampered = String::from_utf8(manifest).unwrap().replace(&format!("sha256:{H}"), &format!("sha256:{}", "2".repeat(64)));
        assert!(refused(Fetched::Files { envelope, signatures, manifest: tampered.into_bytes() }, &[key()], VERSION), "a digest swapped after signing");
    }
}
