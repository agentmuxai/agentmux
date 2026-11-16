// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A second source for the update check, for when GitHub's API limits this computer or fails (#47813).
//!
//! The check learns which release is latest from the `releases/latest` page, which the API's limit doesn't
//! touch, and asks the API for its files and checksums. While the API refuses, a release was known by its tag
//! alone and could not be downloaded. This asks the release's own download address instead: the signed
//! envelope, its signature and the manifest (`AgentMux-<version>-envelope.json`, `.sig`, `-manifest.json`),
//! which are not part of the API's limit. Only GitHub's release-download address is used as a source.
//!
//! What it guarantees (each has a test below):
//! - **Nothing older or equal is offered.** The envelope's version must be the tag the check found, and the tag
//!   must be strictly newer than the running build. An equal or older tag is "nothing new": not an error, not
//!   a download, and nothing is fetched for it.
//! - **One verifier.** The file list is read from a manifest that passes the downloader's own check
//!   (`signature::judge` and `decide`, with the same keys and signature rules, a second signature included),
//!   so the check can't offer a file the download would refuse. Where the two could disagree on one input,
//!   a test fails.
//! - **Bounded.** It runs only when the API is limited or failed. A result is kept for its tag, a failed try is
//!   not repeated before the API's limit ends (and never within [`MIN_RETRY_GAP`]), and a manifest that
//!   fails verification is logged once per tag.
//!
//! An unsigned, unverifiable or unreachable answer is no answer: the release stays known by its tag.

use agentmux_common::update_manifest::{verify_release, version_core, UpdateKey, MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS};
use parking_lot::Mutex;

use crate::backend::update::check::{Release, ReleaseAsset};
use crate::backend::update::download::RELEASE_DOWNLOAD_PREFIX;
use crate::backend::update::install::InstallKind;
use crate::backend::update::signature::{decide, fetch_small, file_names, judge};

/// The shortest gap before a failed try for one release is repeated, whatever the API says about its limit.
pub(crate) const MIN_RETRY_GAP_MS: i64 = 5 * 60_000;

/// What the second source made of a release.
#[derive(Debug)]
pub(crate) enum Answer {
    /// The tag isn't newer than the running build: nothing new. Nothing was fetched for it.
    NothingNew,
    /// A release whose file list was read from a manifest that verified.
    Release(Box<Release>),
    /// No answer, and why.
    NoAnswer(String),
}

/// Whether `tag` is strictly newer than the running build `current`. Versions that don't parse are never newer.
pub(crate) fn newer_than(tag: &str, current: &str) -> bool {
    matches!((version_core(tag), version_core(current)), (Some(t), Some(c)) if t > c)
}

/// The decision for `tag`, given what was fetched (`None`: the release has no signed manifest) and the
/// install it is for. `floor` is the oldest version known to upgrade in place, which a manifest doesn't carry.
pub(crate) fn answer(
    keys: &[UpdateKey],
    tag: &str,
    current: &str,
    kind: &InstallKind,
    arch: &str,
    floor: Option<&str>,
    fetched: Option<(&[u8], &str, &[u8])>,
) -> Answer {
    if !newer_than(tag, current) {
        return Answer::NothingNew;
    }
    let version = tag.trim_start_matches('v');
    let Some(fetched) = fetched else {
        return Answer::NoAnswer("the release has no signed manifest".into());
    };
    let manifest = match verify_release(keys, tag, fetched.0, fetched.1, fetched.2) {
        Ok(m) => m,
        Err(e) => return Answer::NoAnswer(e.to_string()),
    };
    let file = kind.asset_name(version, arch).and_then(|name| manifest.file(&name).cloned());
    let Some(file) = file else {
        // A genuine release with nothing for this computer: say so rather than "not available yet".
        return Answer::Release(Box::new(Release::from_signed(tag, None, floor)));
    };
    let asset = ReleaseAsset { url: format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", file.name), name: file.name, size: file.size, sha256: file.sha256 };
    // The downloader's own decision on the same file, so the two can't differ.
    if let Err(why) = decide(version, &judge(keys, tag, Some(fetched), &asset)) {
        return Answer::NoAnswer(why);
    }
    Answer::Release(Box::new(Release::from_signed(tag, Some(&asset), floor)))
}

/// What is kept about a release's tag between checks.
#[derive(Debug, Clone)]
struct Tried {
    tag: String,
    /// A release that verified: kept for this tag (a signed release's files do not change).
    found: Option<Release>,
    /// A failed try is not repeated before this time (unix ms).
    not_before_ms: i64,
    /// The failure was logged already.
    logged: bool,
}

static TRIED: Mutex<Option<Tried>> = Mutex::new(None);

/// What to do about `tag` now.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// A verified release is held for this tag.
    Held,
    /// A try failed lately: wait.
    Wait,
    /// Ask.
    Ask,
}

fn step(tried: Option<&Tried>, tag: &str, now_ms: i64) -> Step {
    match tried.filter(|t| t.tag == tag) {
        Some(t) if t.found.is_some() => Step::Held,
        Some(t) if now_ms < t.not_before_ms => Step::Wait,
        _ => Step::Ask,
    }
}

/// When a failed try for a release may be repeated: not before the API's limit ends, and never sooner than
/// [`MIN_RETRY_GAP_MS`] from now.
pub(crate) fn retry_not_before(limited_until_ms: i64, now_ms: i64) -> i64 {
    limited_until_ms.max(now_ms + MIN_RETRY_GAP_MS)
}

/// One small file of a release, by name; `Ok(None)` when it is not there (404).
async fn get(client: &reqwest::Client, dir: &str, name: &str, cap: usize) -> Result<Option<Vec<u8>>, String> {
    fetch_small(client, &format!("{dir}{name}"), cap).await.map_err(|e| e.to_string())
}

/// The three signed files of `tag`'s release, from its download address. `Ok(None)`: it has none (404).
async fn fetch(client: &reqwest::Client, tag: &str) -> Result<Option<(Vec<u8>, String, Vec<u8>)>, String> {
    let (envelope, signature, manifest) = file_names(tag.trim_start_matches('v'));
    let dir = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/");
    let Some(envelope) = get(client, &dir, &envelope, MAX_ENVELOPE_BYTES).await? else { return Ok(None) };
    let Some(signature) = get(client, &dir, &signature, MAX_ENVELOPE_BYTES).await? else { return Ok(None) };
    let Some(manifest) = get(client, &dir, &manifest, MAX_MANIFEST_BYTES).await? else { return Ok(None) };
    Ok(Some((envelope, String::from_utf8_lossy(&signature).into_owned(), manifest)))
}

/// What a check gets from the second source.
pub(crate) enum Got {
    NothingNew,
    Release(Release),
    /// No answer: the release stays known by its tag.
    None,
}

/// The release `tag` names, from the signed files at its download address; called only when the API is
/// limited (until `limited_until_ms`, 0 when it isn't, as when it failed) or failed.
pub(crate) async fn release_for(
    client: &reqwest::Client,
    tag: &str,
    current: &str,
    kind: &InstallKind,
    floor: Option<&str>,
    limited_until_ms: i64,
    now_ms: i64,
) -> Got {
    if !newer_than(tag, current) {
        return Got::NothingNew;
    }
    let logged_before = {
        let tried = TRIED.lock();
        match step(tried.as_ref(), tag, now_ms) {
            Step::Held => return tried.as_ref().and_then(|t| t.found.clone()).map_or(Got::None, Got::Release),
            Step::Wait => return Got::None,
            Step::Ask => tried.as_ref().is_some_and(|t| t.tag == tag && t.logged),
        }
    };
    let fetched = fetch(client, tag).await;
    let verdict = match &fetched {
        Ok(Some((e, s, m))) => answer(UPDATE_KEYS, tag, current, kind, std::env::consts::ARCH, floor, Some((e, s, m))),
        Ok(None) => Answer::NoAnswer("the release has no signed manifest at its download address".into()),
        Err(e) => Answer::NoAnswer(e.clone()),
    };
    // A manifest that was fetched and did not verify is worth a warning, once for the tag; a release that has
    // none, or one that could not be reached, is the ordinary case.
    let did_not_verify = matches!(fetched, Ok(Some(_))) && matches!(verdict, Answer::NoAnswer(_));
    let mut tried = TRIED.lock();
    match verdict {
        Answer::NothingNew => Got::NothingNew,
        Answer::Release(r) => {
            *tried = Some(Tried { tag: tag.to_string(), found: Some((*r).clone()), not_before_ms: 0, logged: logged_before });
            tracing::info!(tag, "update check: the release's files came from its signed manifest (GitHub's API is not answering)");
            Got::Release(*r)
        }
        Answer::NoAnswer(why) => {
            if did_not_verify && !logged_before {
                tracing::warn!(tag, %why, "update check: the release's signed manifest did not verify, so its files are not offered");
            }
            *tried = Some(Tried { tag: tag.to_string(), found: None, not_before_ms: retry_not_before(limited_until_ms, now_ms), logged: logged_before || did_not_verify });
            Got::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::update::signature::fixtures::*;

    const NEWER_THAN_FIXTURE: &str = "0.60.0";

    fn run(keys: &[UpdateKey], tag: &str, current: &str, kind: &InstallKind, fetched: Option<(&[u8], &str, &[u8])>) -> Answer {
        answer(keys, tag, current, kind, "x86_64", None, fetched)
    }

    fn released(a: &Answer) -> Option<&Release> {
        match a {
            Answer::Release(r) => Some(r),
            _ => None,
        }
    }

    #[test]
    fn a_tag_equal_to_or_older_than_the_running_build_is_nothing_new_and_a_newer_one_is_a_release() {
        let kind = InstallKind::WindowsSetup;
        let f = Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes()));
        for current in ["0.60.1", "0.60.2", "0.61.0", "1.0.0"] {
            assert!(matches!(run(&keys(), "v0.60.1", current, &kind, f), Answer::NothingNew), "{current}");
        }
        // Not even a failed fetch matters for one that is not newer: there is nothing to ask for.
        assert!(matches!(run(&keys(), "v0.60.1", "0.60.1", &kind, None), Answer::NothingNew));
        assert!(released(&run(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, f)).is_some());
        // A version that doesn't parse is never newer.
        assert!(matches!(run(&keys(), "v0.60.1", "dev", &kind, f), Answer::NothingNew));
        assert!(matches!(run(&keys(), "latest", "0.60.0", &kind, f), Answer::NothingNew));
    }

    #[test]
    fn an_older_signed_release_replayed_under_a_newer_tag_is_no_answer() {
        let kind = InstallKind::WindowsSetup;
        let f = Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes()));
        // The envelope and manifest are v0.60.1's, signed; the tag found is v0.60.2.
        match run(&keys(), "v0.60.2", NEWER_THAN_FIXTURE, &kind, f) {
            Answer::NoAnswer(why) => assert!(why.contains("v0.60.2"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_tag_that_is_not_newer_fetches_nothing() {
        // No network is needed (the client can't reach anything): it must not even try.
        let client = reqwest::Client::builder().build().unwrap();
        let got = release_for(&client, "v0.60.1", "0.60.1", &InstallKind::WindowsSetup, None, 0, 1_000).await;
        assert!(matches!(got, Got::NothingNew));
        assert!(TRIED.lock().as_ref().map_or(true, |t| t.tag != "v0.60.1"));
    }

    #[test]
    fn the_release_it_reads_has_the_signed_file_with_its_checksum_and_the_floor_this_build_knows() {
        let kind = InstallKind::WindowsSetup;
        let a = answer(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", Some("0.55.0"), Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())));
        let status = crate::backend::update::check::status_for(NEWER_THAN_FIXTURE, "stable", released(&a).unwrap(), 1, &kind);
        assert_eq!(status.state, "available");
        assert!(status.can_download, "{status:?}");
        let asset = status.asset.unwrap();
        assert_eq!(asset.name, "AgentMux-0.60.1-x64-setup.exe");
        assert_eq!(asset.sha256, SETUP_SHA);
        assert_eq!(asset.url, format!("{RELEASE_DOWNLOAD_PREFIX}v0.60.1/AgentMux-0.60.1-x64-setup.exe"));
        // An install older than that floor is sent to it first, as with the API's answer.
        let old = crate::backend::update::check::status_for("0.50.0", "stable", released(&a).unwrap(), 1, &kind);
        assert_eq!(old.install_first.as_deref(), Some("0.55.0"));
        assert!(!old.can_download);
    }

    #[test]
    fn a_signed_release_with_no_file_for_this_computer_says_so() {
        let kind = InstallKind::LinuxAppImage;
        let a = answer(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", None, Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())));
        let status = crate::backend::update::check::status_for(NEWER_THAN_FIXTURE, "stable", released(&a).unwrap(), 1, &kind);
        assert!(!status.can_download && status.asset.is_none());
        assert_eq!(status.download_blocked.as_deref(), Some("This release has no file for this computer."));
    }

    /// The same input through the check's reading and through the downloader's own decision (`gate`'s
    /// `judge` + `decide`) must agree: a release the check offers is one the download accepts, and the
    /// reverse. Each case is one input the two share.
    #[test]
    fn the_check_and_the_downloader_decide_the_same_on_every_input() {
        let kind = InstallKind::WindowsSetup;
        let name = "AgentMux-0.60.1-x64-setup.exe";
        let k2_only = [UpdateKey { id: "k2", public: KEY }];
        let both = [UpdateKey { id: "k1", public: [9; 32] }, UpdateKey { id: "k2", public: KEY }];
        let second = format!("k1 AAAA\nk2 {}", SIG.trim_start_matches("k1 "));
        let second_only = SIG.replacen("k1 ", "k2 ", 1);
        let tampered_manifest = MANIFEST.replace("\"size\": 100", "\"size\": 101");
        let other_envelope = ENVELOPE.replace("v0.60.1", "v0.60.2");
        let cases: Vec<(&str, Vec<UpdateKey>, &str, &str, &str)> = vec![
            ("good", keys(), "v0.60.1", SIG, MANIFEST),
            ("a second signature line, the first one not valid", both.to_vec(), "v0.60.1", second.as_str(), MANIFEST),
            ("only the second key's signature, and that key pinned", k2_only.to_vec(), "v0.60.1", second_only.as_str(), MANIFEST),
            ("only the second key's signature, key not pinned", keys(), "v0.60.1", second_only.as_str(), MANIFEST),
            ("another key", vec![UpdateKey { id: "k1", public: [9; 32] }], "v0.60.1", SIG, MANIFEST),
            ("no signature at all", keys(), "v0.60.1", "", MANIFEST),
            ("a tampered manifest", keys(), "v0.60.1", SIG, tampered_manifest.as_str()),
            ("another release's tag", keys(), "v0.60.2", SIG, MANIFEST),
            ("an envelope for another tag", keys(), "v0.60.1", SIG, MANIFEST),
        ];
        for (label, keys, tag, sig, manifest) in cases {
            let envelope = if label == "an envelope for another tag" { other_envelope.as_str() } else { ENVELOPE };
            let fetched = Some((envelope.as_bytes(), sig, manifest.as_bytes()));
            let checked = run(&keys, tag, NEWER_THAN_FIXTURE, &kind, fetched);
            // What the downloader does with that file from that release.
            let asset = ReleaseAsset { name: name.into(), size: 100, url: format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{name}"), sha256: SETUP_SHA.into() };
            let download_ok = decide(tag.trim_start_matches('v'), &judge(&keys, tag, fetched, &asset)).is_ok();
            assert_eq!(released(&checked).is_some_and(|r| r.has_signed_file(name)), download_ok, "{label}: {checked:?}");
        }
    }

    fn tried(tag: &str, found: bool, not_before_ms: i64) -> Tried {
        Tried { tag: tag.into(), found: found.then(|| Release::from_signed(tag, None, None)), not_before_ms, logged: false }
    }

    #[test]
    fn it_asks_once_per_release_keeps_what_verified_and_waits_out_a_failure() {
        // Nothing tried: ask. Another release's try says nothing about this one.
        assert_eq!(step(None, "v0.60.31", 100), Step::Ask);
        assert_eq!(step(Some(&tried("v0.60.30", false, 10_000)), "v0.60.31", 100), Step::Ask);
        // A verified release is held for its tag, however long ago.
        assert_eq!(step(Some(&tried("v0.60.31", true, 0)), "v0.60.31", 99_999_999), Step::Held);
        // A failed try waits, then is allowed again.
        assert_eq!(step(Some(&tried("v0.60.31", false, 10_000)), "v0.60.31", 9_999), Step::Wait);
        assert_eq!(step(Some(&tried("v0.60.31", false, 10_000)), "v0.60.31", 10_000), Step::Ask);
    }

    #[test]
    fn a_failed_try_is_not_repeated_before_the_limit_ends_and_never_faster_than_the_gap() {
        let now = 1_000_000;
        // The API's limit ends in an hour: the second source waits as long.
        assert_eq!(retry_not_before(now + 3_600_000, now), now + 3_600_000);
        // The API failed (no limit), or the limit ends in seconds: still the minimum gap.
        assert_eq!(retry_not_before(0, now), now + MIN_RETRY_GAP_MS);
        assert_eq!(retry_not_before(now + 5_000, now), now + MIN_RETRY_GAP_MS);
        assert!(MIN_RETRY_GAP_MS >= 60_000, "never faster than the timed re-check after a limit");
    }
}
