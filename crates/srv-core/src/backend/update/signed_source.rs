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
//! - **The release's own upgrade floor, from what the pipeline signed.** A manifest may carry an `upgrade.floor`
//!   (`SPEC_UPGRADE_FLOOR_IN_SIGNED_MANIFEST`). When it carries a usable one, the floor used is the higher of that
//!   and this build's own (`BUILD_FLOOR`), and neither the notes' floor line nor the raw `UPGRADE_FLOOR` file is
//!   asked for. A manifest with none (or an unusable one, logged once per tag) falls back to what was done before:
//!   the repo's `UPGRADE_FLOOR` file at the release's tag, which the API's answer vouches for no better (GitHub
//!   over TLS, unsigned). The floor used is then the higher of this build's own and that file's, so a read can only
//!   block a download, never allow one. A tag that isn't plainly `v<digits>.<digits>.<digits>` builds no address,
//!   and a file that can't be read or isn't a plain version is no answer.
//! - **Bounded.** It runs only when the API is limited or failed. A result is kept for its tag, a failed try is
//!   not repeated before the API's limit ends (and never within [`MIN_RETRY_GAP_MS`]), and a manifest or floor
//!   that fails is logged once per tag.
//!
//! An unsigned, unverifiable or unreachable answer is no answer: the release stays known by its tag.

use agentmux_common::update_manifest::{verify_release, version_core, SignedFloor, UpdateKey, MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, UPDATE_KEYS};
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

/// Where `tag`'s upgrade floor is read: the repo's `UPGRADE_FLOOR` at that tag.
const FLOOR_FILE_PREFIX: &str = "https://raw.githubusercontent.com/agentmuxai/agentmux/";

/// A floor file is a version and a newline; anything longer is not one.
const MAX_FLOOR_BYTES: usize = 64;

/// `<digits>.<digits>.<digits>` and nothing else.
fn plain_version(s: &str) -> bool {
    let mut parts = s.split('.');
    let number = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()));
    number(parts.next()) && number(parts.next()) && number(parts.next()) && parts.next().is_none()
}

/// The address of the floor file at `tag`, only for a tag of exactly `v<digits>.<digits>.<digits>`.
pub(crate) fn floor_url(tag: &str) -> Option<String> {
    plain_version(tag.strip_prefix('v')?).then(|| format!("{FLOOR_FILE_PREFIX}{tag}/UPGRADE_FLOOR"))
}

/// The floor to use for a release: the higher of this build's own (`build`) and what the floor file at the
/// release's tag says (`file`: `Ok(None)` when there is no such file). A file that can't be read, isn't there or
/// isn't a plain version is an error: no floor, so no answer. A lower file never lowers the build's floor.
pub(crate) fn floor_for(build: Option<&str>, file: Result<Option<Vec<u8>>, String>) -> Result<String, String> {
    let bytes = file.map_err(|e| format!("the release's upgrade floor couldn't be read: {e}"))?;
    let bytes = bytes.ok_or_else(|| "the release has no upgrade floor file at its tag".to_string())?;
    let read = std::str::from_utf8(&bytes).map(str::trim).ok().filter(|t| plain_version(t));
    let Some(read) = read else { return Err("the release's upgrade floor isn't a version".to_string()) };
    Ok(match build.filter(|b| version_core(b) >= version_core(read)) {
        Some(build) => build.to_string(),
        None => read.to_string(),
    })
}

/// The floor in force when the verified manifest carries a usable one: the higher of that and this build's own
/// (`build`), which a signed floor never lowers. `None` when the manifest has none that can be used: the floor is then
/// read the way it was before the manifest carried one ([`floor_for`]).
pub(crate) fn signed_floor_in_force(build: Option<&str>, signed: &SignedFloor) -> Option<String> {
    let SignedFloor::Valid(signed) = signed else { return None };
    Some(match build.filter(|b| version_core(b) >= version_core(signed)) {
        Some(build) => build.to_string(),
        None => signed.clone(),
    })
}

/// The floor for a release: the signed one when the manifest carries a usable one, else what the floor file at the
/// release's tag says. `read_file` is asked only in the second case, so a signed floor never needs, and never reads,
/// the unsigned file.
pub(crate) async fn resolve_floor<F, Fut>(build: Option<&str>, signed: &SignedFloor, read_file: F) -> Result<String, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<Vec<u8>>, String>>,
{
    match signed_floor_in_force(build, signed) {
        Some(floor) => Ok(floor),
        None => floor_for(build, read_file().await),
    }
}

/// What the manifest at `fetched` signs about the upgrade floor, once it verifies; `Absent` for anything that does not
/// (no files, a fetch that failed, a manifest that doesn't verify: those are the verdict's to report).
pub(crate) fn signed_floor_of(keys: &[UpdateKey], tag: &str, fetched: &Fetched) -> SignedFloor {
    floor_read(keys, tag, fetched).0
}

/// [`signed_floor_of`], and why when signed files were fetched and did not verify. That is not the ordinary case (a
/// release with no signed files is, and a fetch that failed is a network matter): it is what a tampered or mis-signed
/// release looks like, so the caller says so, once per tag. The floor is `Absent` then, as before.
pub(crate) fn floor_read(keys: &[UpdateKey], tag: &str, fetched: &Fetched) -> (SignedFloor, Option<String>) {
    match fetched {
        Ok(Some((e, s, m))) => match verify_release(keys, tag, e, s, m) {
            Ok(manifest) => (manifest.signed_floor(), None),
            Err(why) => (SignedFloor::Absent, Some(why.to_string())),
        },
        _ => (SignedFloor::Absent, None),
    }
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

/// What was fetched for a release: its three signed files (`Ok(None)`: it has none), or why not.
type Fetched = Result<Option<(Vec<u8>, String, Vec<u8>)>, String>;

/// The answer for `tag` from its floor (or why there is none) and what was fetched: no floor is no answer,
/// whatever the signed files say.
fn verdict(keys: &[UpdateKey], tag: &str, current: &str, kind: &InstallKind, arch: &str, floor: &Result<String, String>, fetched: &Fetched) -> Answer {
    match (floor, fetched) {
        (Err(why), _) => Answer::NoAnswer(why.clone()),
        (Ok(floor), Ok(Some((e, s, m)))) => answer(keys, tag, current, kind, arch, Some(floor), Some((e, s, m))),
        (Ok(_), Ok(None)) => Answer::NoAnswer("the release has no signed manifest at its download address".into()),
        (Ok(_), Err(e)) => Answer::NoAnswer(e.clone()),
    }
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
async fn fetch(client: &reqwest::Client, tag: &str) -> Fetched {
    let (envelope, signature, manifest) = file_names(tag.trim_start_matches('v'));
    let dir = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/");
    let Some(envelope) = get(client, &dir, &envelope, MAX_ENVELOPE_BYTES).await? else { return Ok(None) };
    let Some(signature) = get(client, &dir, &signature, MAX_ENVELOPE_BYTES).await? else { return Ok(None) };
    let Some(manifest) = get(client, &dir, &manifest, MAX_MANIFEST_BYTES).await? else { return Ok(None) };
    Ok(Some((envelope, String::from_utf8_lossy(&signature).into_owned(), manifest)))
}

/// What was learned of a release's signed floor, kept for its tag: a signed release's files do not change, so a manifest
/// that was read is read once; a failed fetch is not repeated within [`MIN_RETRY_GAP_MS`].
#[derive(Debug, Clone)]
struct FloorTried {
    tag: String,
    /// The floor the verified manifest carries (`Absent` also for a release with no signed manifest, or one that did not
    /// verify); `None` while only a fetch that failed is known.
    floor: Option<SignedFloor>,
    not_before_ms: i64,
    logged: bool,
}

static SIGNED_FLOOR: Mutex<Option<FloorTried>> = Mutex::new(None);

/// The upgrade floor `tag`'s signed manifest carries, for a check that learned the release from GitHub's API: the same
/// verifier as the download's, asked once per tag. `Absent` when there is none to use (no manifest, one that does not
/// verify, a fetch that failed): the floor is then what the release notes say, as before.
pub(crate) async fn signed_floor_for(client: &reqwest::Client, tag: &str, now_ms: i64) -> SignedFloor {
    let logged_before = {
        let held = SIGNED_FLOOR.lock();
        match held.as_ref().filter(|t| t.tag == tag) {
            Some(t) if t.floor.is_some() => return t.floor.clone().unwrap_or(SignedFloor::Absent),
            Some(t) if now_ms < t.not_before_ms => return SignedFloor::Absent,
            Some(t) => t.logged,
            None => false,
        }
    };
    let fetched = fetch(client, tag).await;
    let (signed, unverified) = floor_read(UPDATE_KEYS, tag, &fetched);
    let mut logged = logged_before;
    if !logged_before {
        if let Some(why) = &unverified {
            // A red flag, not the ordinary case: the release's signed files were there and did not verify. The floor falls
            // back to the notes' (and the download, which verifies the same files, will refuse the release).
            tracing::warn!(tag, %why, "update check: the release's signed manifest did not verify, so its signed upgrade floor is not used (a tampered or mis-signed release?)");
            logged = true;
        } else if let SignedFloor::Unusable(why) = &signed {
            tracing::warn!(tag, %why, "update check: the release's signed upgrade floor isn't usable; its notes' floor is used");
            logged = true;
        }
    }
    // A fetch that failed says nothing about the release: try again later. Anything read (or a 404) is kept.
    let floor = fetched.is_ok().then(|| signed.clone());
    *SIGNED_FLOOR.lock() = Some(FloorTried { tag: tag.to_string(), floor, not_before_ms: now_ms + MIN_RETRY_GAP_MS, logged });
    signed
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
    build_floor: Option<&str>,
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
    // The signed files first: their floor, when they carry a usable one, is the floor, and the unsigned file is not asked for.
    let fetched = fetch(client, tag).await;
    let signed = signed_floor_of(UPDATE_KEYS, tag, &fetched);
    let floor = resolve_floor(build_floor, &signed, || async {
        match floor_url(tag) {
            Some(url) => fetch_small(client, &url, MAX_FLOOR_BYTES).await.map_err(|e| e.to_string()),
            None => Err(format!("{tag} isn't a plain release tag")),
        }
    })
    .await;
    let unusable = match &signed {
        SignedFloor::Unusable(why) if !logged_before => Some(why.clone()),
        _ => None,
    };
    if let Some(why) = &unusable {
        tracing::warn!(tag, %why, "update check: the release's signed upgrade floor isn't usable; its notes' or floor file's is used");
    }
    let logged_before = logged_before || unusable.is_some();
    let verdict = verdict(UPDATE_KEYS, tag, current, kind, std::env::consts::ARCH, &floor, &fetched);
    // A manifest that was fetched and did not verify, or a floor that could not be used, is worth a warning,
    // once for the tag; a release that has no signed files, or one that could not be reached, is the ordinary case.
    let did_not_verify = matches!(verdict, Answer::NoAnswer(_)) && (floor.is_err() || matches!(fetched, Ok(Some(_))));
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
                tracing::warn!(tag, %why, "update check: the release's signed manifest or upgrade floor could not be used, so its files are not offered");
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
        let status = crate::backend::update::check::status_for_arch(NEWER_THAN_FIXTURE, "stable", released(&a).unwrap(), 1, &kind, "x86_64");
        assert_eq!(status.state, "available");
        assert!(status.can_download, "{status:?}");
        let asset = status.asset.unwrap();
        assert_eq!(asset.name, "AgentMux-0.60.1-x64-setup.exe");
        assert_eq!(asset.sha256, SETUP_SHA);
        assert_eq!(asset.url, format!("{RELEASE_DOWNLOAD_PREFIX}v0.60.1/AgentMux-0.60.1-x64-setup.exe"));
        // An install older than that floor is sent to it first, as with the API's answer.
        let old = crate::backend::update::check::status_for_arch("0.50.0", "stable", released(&a).unwrap(), 1, &kind, "x86_64");
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

    #[test]
    fn the_floor_is_read_only_for_a_plain_release_tag_at_that_tag() {
        assert_eq!(floor_url("v0.60.31").as_deref(), Some("https://raw.githubusercontent.com/agentmuxai/agentmux/v0.60.31/UPGRADE_FLOOR"));
        for tag in ["", "v", "0.60.31", "v0.60", "v0.60.31.1", "v0.60.x", "v0.60.31-rc1", "v0.60.31/../main", "v0.60.31?x=1", "v0.60.31 ", "vv0.60.31", "v0.60.-1", "latest", "v1234567890.0.0"] {
            assert_eq!(floor_url(tag), None, "{tag:?}");
        }
    }

    #[test]
    fn a_floor_at_the_tag_above_the_running_build_blocks_the_download_and_names_what_to_install_first() {
        let kind = InstallKind::WindowsSetup;
        // This build's floor is 0.55.0; the release's file says 0.60.1, above the running 0.60.0.
        let floor = floor_for(Some("0.55.0"), Ok(Some(b"0.60.1\n".to_vec())));
        assert_eq!(floor.as_deref(), Ok("0.60.1"));
        let a = answer(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", floor.as_deref().ok(), Some((ENVELOPE.as_bytes(), SIG, MANIFEST.as_bytes())));
        let status = crate::backend::update::check::status_for(NEWER_THAN_FIXTURE, "stable", released(&a).unwrap(), 1, &kind);
        assert_eq!(status.install_first.as_deref(), Some("0.60.1"));
        assert!(!status.can_download, "{status:?}");
    }

    #[test]
    fn a_floor_that_cannot_be_read_offers_nothing_even_with_good_signed_files() {
        let kind = InstallKind::WindowsSetup;
        let good = Ok(Some((ENVELOPE.as_bytes().to_vec(), SIG.to_string(), MANIFEST.as_bytes().to_vec())));
        let unreadable: Vec<Result<Option<Vec<u8>>, String>> = vec![
            Err("HTTP 503".into()),
            Ok(None),
            Ok(Some(vec![])),
            Ok(Some(b"\n".to_vec())),
            Ok(Some(b"latest".to_vec())),
            Ok(Some(b"0.55".to_vec())),
            Ok(Some(b"0.55.0.1".to_vec())),
            Ok(Some(b"v0.55.0".to_vec())),
            Ok(Some(b"0.55.0-rc1".to_vec())),
            Ok(Some(b"0.55.0\n0.56.0".to_vec())),
            Ok(Some(vec![0xff, 0xfe, b'0'])),
            Ok(Some(b"<html>404</html>".to_vec())),
        ];
        for file in unreadable {
            let label = format!("{file:?}");
            let floor = floor_for(Some("0.55.0"), file);
            assert!(floor.is_err(), "{label}");
            match verdict(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", &floor, &good) {
                Answer::NoAnswer(_) => {}
                other => panic!("{label}: {other:?}"),
            }
        }
        // With a floor, the same files are an answer.
        let ok = floor_for(Some("0.55.0"), Ok(Some(b"0.55.0\n".to_vec())));
        assert!(released(&verdict(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", &ok, &good)).is_some());
    }

    #[test]
    fn a_floor_file_lower_than_this_builds_floor_does_not_lower_it() {
        assert_eq!(floor_for(Some("0.55.0"), Ok(Some(b"0.40.0".to_vec()))).as_deref(), Ok("0.55.0"));
        assert_eq!(floor_for(Some("0.55.0"), Ok(Some(b"0.55.0\r\n".to_vec()))).as_deref(), Ok("0.55.0"));
        assert_eq!(floor_for(Some("0.55.0"), Ok(Some(b"0.56.0".to_vec()))).as_deref(), Ok("0.56.0"));
        // A build floor that isn't a version never wins.
        assert_eq!(floor_for(Some("junk"), Ok(Some(b"0.40.0".to_vec()))).as_deref(), Ok("0.40.0"));
        assert_eq!(floor_for(None, Ok(Some(b"0.40.0".to_vec()))).as_deref(), Ok("0.40.0"));
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

    // Two releases as `scripts/release-manifest.mjs build` makes them with an `upgrade` object, signed with the same
    // seed-7 test key as the fixtures above, copied byte for byte: A has floor 0.55.0 (and a rollout flag), B has 0.60.1.
    const A_MANIFEST: &str = "{\n  \"format\": 1,\n  \"version\": \"0.60.1\",\n  \"tag\": \"v0.60.1\",\n  \"files\": [\n    {\n      \"name\": \"AgentMux-0.60.1-x64-setup.exe\",\n      \"size\": 5,\n      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"\n    },\n    {\n      \"name\": \"agentmux-0.60.1-x64-portable.7z\",\n      \"size\": 8,\n      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"\n    }\n  ],\n  \"upgrade\": {\n    \"floor\": \"0.55.0\",\n    \"shared_store_changed\": true\n  }\n}\n";
    const A_ENVELOPE: &str = "{\n  \"format\": 1,\n  \"tag\": \"v0.60.1\",\n  \"manifest_sha256\": \"2038d37391a59d7b22bb0bee89eb31b6f94df10d4730389b79808fb32a52272f\",\n  \"created_at\": \"2026-10-25T12:00:00Z\"\n}\n";
    const A_SIG: &str = "k1 wf8ru0NTcp3G7GCZff/1dX9O2SEd7nSHuacY058sSrcQHFkpidkkwklrjqb1hJOr61diLh/2rRGg32Gnn5whDw==\n\n";
    const B_MANIFEST: &str = "{\n  \"format\": 1,\n  \"version\": \"0.60.1\",\n  \"tag\": \"v0.60.1\",\n  \"files\": [\n    {\n      \"name\": \"AgentMux-0.60.1-x64-setup.exe\",\n      \"size\": 5,\n      \"sha256\": \"8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73\"\n    },\n    {\n      \"name\": \"agentmux-0.60.1-x64-portable.7z\",\n      \"size\": 8,\n      \"sha256\": \"01e782826ae5182220bd6158f883d01ceb1bce659dc020e7c511f802a9aa7737\"\n    }\n  ],\n  \"upgrade\": {\n    \"floor\": \"0.60.1\"\n  }\n}\n";
    const B_ENVELOPE: &str = "{\n  \"format\": 1,\n  \"tag\": \"v0.60.1\",\n  \"manifest_sha256\": \"2356952b4c0debe50af877ac299efa107dc851c865b6b96330af581007d5d519\",\n  \"created_at\": \"2026-10-25T12:00:00Z\"\n}\n";
    const B_SIG: &str = "k1 7hvcUAvIjUfGzcTIOtS5zBD/0/k2VzeyWBR3akHyu6Lzp8kiydOj582QrSRoKVahOHMBnWdN6E+DFbKBgFeKAg==\n\n";

    fn fetched(envelope: &str, sig: &str, manifest: &str) -> Fetched {
        Ok(Some((envelope.as_bytes().to_vec(), sig.to_string(), manifest.as_bytes().to_vec())))
    }

    #[test]
    fn the_floor_is_read_from_the_manifest_that_verified_and_from_nothing_else() {
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &fetched(A_ENVELOPE, A_SIG, A_MANIFEST)), SignedFloor::Valid("0.55.0".into()));
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &fetched(B_ENVELOPE, B_SIG, B_MANIFEST)), SignedFloor::Valid("0.60.1".into()));
        // A manifest with none says nothing about it.
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &fetched(ENVELOPE, SIG, MANIFEST)), SignedFloor::Absent);
        // One that does not verify, another release's, a release with no signed files and a failed fetch: nothing to use.
        let tampered = A_MANIFEST.replace("0.55.0", "0.40.0");
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &fetched(A_ENVELOPE, A_SIG, &tampered)), SignedFloor::Absent);
        assert_eq!(signed_floor_of(&keys(), "v0.60.2", &fetched(A_ENVELOPE, A_SIG, A_MANIFEST)), SignedFloor::Absent);
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &fetched(A_ENVELOPE, "k9 AAAA\n", A_MANIFEST)), SignedFloor::Absent);
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &Ok(None)), SignedFloor::Absent);
        assert_eq!(signed_floor_of(&keys(), "v0.60.1", &Err("no network".into())), SignedFloor::Absent);
    }

    #[test]
    fn the_floor_in_force_is_the_higher_of_the_signed_floor_and_the_builds() {
        let signed = |f: &str| SignedFloor::Valid(f.into());
        // Above the build's: it raises it. Below, or equal: it never lowers it.
        assert_eq!(signed_floor_in_force(Some("0.55.0"), &signed("0.58.0")).as_deref(), Some("0.58.0"));
        assert_eq!(signed_floor_in_force(Some("0.55.0"), &signed("0.40.0")).as_deref(), Some("0.55.0"));
        assert_eq!(signed_floor_in_force(Some("0.55.0"), &signed("0.55.0")).as_deref(), Some("0.55.0"));
        // A build floor that isn't a version never wins; none at all is the signed one.
        assert_eq!(signed_floor_in_force(Some("junk"), &signed("0.40.0")).as_deref(), Some("0.40.0"));
        assert_eq!(signed_floor_in_force(None, &signed("0.40.0")).as_deref(), Some("0.40.0"));
        // No usable signed floor: nothing in force from it.
        assert_eq!(signed_floor_in_force(Some("0.55.0"), &SignedFloor::Absent), None);
        assert_eq!(signed_floor_in_force(Some("0.55.0"), &SignedFloor::Unusable("x".into())), None);
    }

    #[tokio::test]
    async fn a_signed_floor_is_used_without_asking_for_the_unsigned_file() {
        let asked = std::sync::atomic::AtomicBool::new(false);
        let spy = || async {
            asked.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(b"0.59.0\n".to_vec()))
        };
        // The file would say 0.59.0, above the signed 0.58.0: it is not consulted, so the signed one is the floor.
        let floor = resolve_floor(Some("0.55.0"), &SignedFloor::Valid("0.58.0".into()), spy).await;
        assert_eq!(floor.as_deref(), Ok("0.58.0"));
        assert!(!asked.load(std::sync::atomic::Ordering::SeqCst), "the floor file was asked for");
    }

    #[tokio::test]
    async fn without_a_usable_signed_floor_the_floor_is_read_as_it_was() {
        for signed in [SignedFloor::Absent, SignedFloor::Unusable("upgrade.floor isn't a plain version".into())] {
            let asked = std::sync::atomic::AtomicBool::new(false);
            let spy = || async {
                asked.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Some(b"0.56.0\n".to_vec()))
            };
            assert_eq!(resolve_floor(Some("0.55.0"), &signed, spy).await.as_deref(), Ok("0.56.0"), "{signed:?}");
            assert!(asked.load(std::sync::atomic::Ordering::SeqCst), "{signed:?}");
            // And a file that can't be used is still no answer.
            assert!(resolve_floor(Some("0.55.0"), &signed, || async { Ok(None) }).await.is_err(), "{signed:?}");
            assert!(resolve_floor(Some("0.55.0"), &signed, || async { Err("offline".to_string()) }).await.is_err(), "{signed:?}");
        }
    }

    #[test]
    fn a_signed_floor_above_the_running_build_blocks_the_download_and_names_what_to_install_first() {
        let kind = InstallKind::WindowsSetup;
        let signed = signed_floor_of(&keys(), "v0.60.1", &fetched(B_ENVELOPE, B_SIG, B_MANIFEST));
        let floor = signed_floor_in_force(Some("0.55.0"), &signed);
        assert_eq!(floor.as_deref(), Some("0.60.1"));
        // The running 0.60.0 is below the signed 0.60.1: no direct upgrade, and the reason names the version to install first.
        let a = answer(&keys(), "v0.60.1", NEWER_THAN_FIXTURE, &kind, "x86_64", floor.as_deref(), Some((B_ENVELOPE.as_bytes(), B_SIG, B_MANIFEST.as_bytes())));
        let status = crate::backend::update::check::status_for_arch(NEWER_THAN_FIXTURE, "stable", released(&a).unwrap(), 1, &kind, "x86_64");
        assert_eq!(status.install_first.as_deref(), Some("0.60.1"));
        assert!(!status.can_download, "{status:?}");
        assert_eq!(
            crate::backend::update::check::install_first_message("0.60.0", "0.60.1", "0.60.1"),
            "This AgentMux (0.60.0) is older than 0.60.1 can upgrade from; install 0.60.1 first, then upgrade."
        );
    }

    #[test]
    fn the_signed_floor_replaces_the_notes_floor_line_and_the_notes_stay_readable() {
        use crate::backend::update::check::{release_from_tag, status_for_arch};
        let kind = InstallKind::WindowsSetup;
        let notes = release_from_tag("v0.60.1", None).with_body("- Faster startup\n<!-- agentmux:min-upgrade-from=0.50.0 -->");
        // The notes say 0.50.0; the signed floor says 0.58.0: the signed one is the floor, and the notes lose only the line.
        let with = notes.clone().with_floor("0.58.0");
        let status = status_for_arch("0.57.0", "stable", &with, 1, &kind, "x86_64");
        assert_eq!(status.install_first.as_deref(), Some("0.58.0"));
        assert_eq!(status.notes.as_deref(), Some("- Faster startup"));
        // A signed floor lower than the notes' wins too (it is the pipeline's own word for this release).
        let lower = notes.with_floor("0.40.0");
        assert_eq!(status_for_arch("0.45.0", "stable", &lower, 1, &kind, "x86_64").install_first, None);
        // No notes at all: just the floor.
        let bare = release_from_tag("v0.60.1", None).with_floor("0.58.0");
        assert_eq!(status_for_arch("0.57.0", "stable", &bare, 1, &kind, "x86_64").install_first.as_deref(), Some("0.58.0"));
    }

    /// The download's gate takes the floor from the manifest it verified, not from what the check saw: with a check that read
    /// no floor (its manifest fetch failed, or the notes' floor was used), a manifest signing a floor above the running
    /// build still refuses.
    #[test]
    fn the_downloads_gate_refuses_below_the_signed_floor_whatever_the_check_saw() {
        use crate::backend::update::signature::{download_verdict, judge_with_floor};
        let setup = ReleaseAsset { name: "AgentMux-0.60.1-x64-setup.exe".into(), size: 5, url: String::new(), sha256: "8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73".into() };
        let gate = |running: &str, build: Option<&str>, e: &str, s: &str, m: &str| {
            let (outcome, floor) = judge_with_floor(&keys(), "v0.60.1", Some((e.as_bytes(), s, m.as_bytes())), &setup);
            download_verdict(running, "0.60.1", build, &outcome, &floor)
        };
        // Manifest B (floor 0.60.1), running 0.60.0: refused, naming 0.60.1, in the message the check gives.
        let refusal = gate("0.60.0", Some("0.55.0"), B_ENVELOPE, B_SIG, B_MANIFEST).unwrap_err();
        assert_eq!(refusal, crate::backend::update::check::install_first_message("0.60.0", "0.60.1", "0.60.1"));
        assert!(refusal.contains("install 0.60.1 first"), "{refusal}");
        // At the floor, or above it: allowed.
        assert!(gate("0.60.1", Some("0.55.0"), B_ENVELOPE, B_SIG, B_MANIFEST).is_ok());
        // Manifest A (floor 0.55.0): running 0.54.0 is refused, 0.55.0 is allowed.
        assert!(gate("0.54.0", Some("0.40.0"), A_ENVELOPE, A_SIG, A_MANIFEST).unwrap_err().contains("install 0.55.0 first"));
        assert!(gate("0.55.0", Some("0.40.0"), A_ENVELOPE, A_SIG, A_MANIFEST).is_ok());
        // This build's own floor, when higher than the signed one, is the one in force (a signed floor never lowers it).
        assert!(gate("0.57.0", Some("0.58.0"), A_ENVELOPE, A_SIG, A_MANIFEST).unwrap_err().contains("install 0.58.0 first"));
        // A manifest that does not verify is refused for its signature, never read for a floor.
        let tampered = B_MANIFEST.replace("\"size\": 5", "\"size\": 6");
        assert!(gate("0.60.1", Some("0.55.0"), B_ENVELOPE, B_SIG, &tampered).unwrap_err().contains("signature"));
    }

    #[test]
    fn a_floor_that_cannot_be_used_adds_nothing_to_the_gate() {
        use crate::backend::update::signature::enforce_floor;
        for floor in [SignedFloor::Absent, SignedFloor::Unusable("x".into())] {
            assert!(enforce_floor("0.10.0", "0.60.1", Some("0.55.0"), &floor).is_ok());
        }
        // A running version that can't be read is below any floor, as in the check.
        assert!(enforce_floor("junk", "0.60.1", Some("0.55.0"), &SignedFloor::Valid("0.60.1".into())).is_err());
    }

    #[test]
    fn signed_files_that_were_fetched_and_did_not_verify_are_reported_once_as_unverified() {
        let tampered = B_MANIFEST.replace("\"size\": 5", "\"size\": 6");
        let (floor, why) = floor_read(&keys(), "v0.60.1", &fetched(B_ENVELOPE, B_SIG, &tampered));
        assert_eq!(floor, SignedFloor::Absent);
        assert!(why.is_some_and(|w| !w.is_empty()));
        // The ordinary cases say nothing: a good manifest, no signed files, and a fetch that failed.
        assert_eq!(floor_read(&keys(), "v0.60.1", &fetched(B_ENVELOPE, B_SIG, B_MANIFEST)), (SignedFloor::Valid("0.60.1".into()), None));
        assert_eq!(floor_read(&keys(), "v0.60.1", &Ok(None)), (SignedFloor::Absent, None));
        assert_eq!(floor_read(&keys(), "v0.60.1", &Err("HTTP 503".into())), (SignedFloor::Absent, None));
    }
}
