// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host-global record of this machine's LAN-enabled instances, so the one
//! instance that holds the UDP probe port can list the others as `siblings`
//! in its reply (`docs/specs/SPEC_LAN_FLEET_FEED_2026_10_03.md` §3, which
//! links the mobile spec that owns the wire contract).
//!
//! One `<channel>.json` per instance under
//! `registry::resolve_shared_lan_instances_dir()`, written only while LAN
//! discovery runs. It carries the instance's `lan_key`, which that instance
//! already broadcasts to the LAN over mDNS, and never the full `auth_key`.
//! A channel with LAN off writes nothing, so it is never listed.

use super::*;
use std::path::{Path, PathBuf};

/// How often a running instance rewrites its record.
pub(super) const REWRITE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
/// A record not rewritten for this long (three missed rewrites) is not listed.
pub(super) const STALE_AFTER_MS: u64 = 60_000;
/// Siblings listed in one reply, before the byte budget below applies.
pub(super) const MAX_SIBLINGS: usize = 16;
/// Serialized size a probe reply with siblings is kept under: a single
/// unfragmented datagram on an Ethernet/Wi-Fi MTU of 1500, with headroom.
pub(super) const MAX_REPLY_BYTES: usize = 1400;
/// Directory entries looked at per read. Every file here is written by a
/// local process, but a full disk of junk must not stall a probe reply.
const MAX_FILES_SCANNED: usize = 64;
/// A real record is about 200 bytes.
const MAX_FILE_BYTES: u64 = 4096;
const MAX_CHANNEL_LEN: usize = 64;
const MAX_KEY_LEN: usize = 256;
const MAX_VERSION_LEN: usize = 64;

/// One instance's record, as stored on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct InstanceRecord {
    pub channel: String,
    pub port: u16,
    pub lan_key: String,
    pub version: String,
    pub pid: u32,
    pub updated_at_ms: u64,
}

/// The file a channel's record lives in: a readable stem, reduced to
/// `[a-z0-9_-]` (plus other Unicode letters and digits) and capped so it can
/// never name a path outside the directory, then 8 hex chars of a SHA-256 of
/// the exact channel name. The slug alone is lossy (`Foo` and `foo`, or
/// `foo bar` and `foo_bar`, share one), and two LAN-enabled channels sharing
/// a file would overwrite each other's record (#4297). The real
/// name stays in the file.
pub(super) fn record_file_name(channel: &str) -> String {
    use sha2::{Digest, Sha256};
    let stem: String = agentmux_common::slug::file_stem(channel)
        .chars()
        .take(MAX_CHANNEL_LEN)
        .collect();
    let digest = Sha256::digest(channel.as_bytes());
    let tag: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    let stem = if stem.is_empty() { "_".to_string() } else { stem };
    format!("{stem}-{tag}.json")
}

/// Write `record` atomically (temp file, then rename) with mode 0600 on Unix
/// at open time, the same care `reactive::registry::write_entry_file` takes
/// for the shared registry, which also holds keys. On Windows the file
/// inherits the user-only ACL of the user's own profile directory.
pub(super) fn write_record(dir: &Path, record: &InstanceRecord) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(record_file_name(&record.channel));
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp_path)?;
    if let Err(e) = f.write_all(&json) {
        drop(f);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    drop(f);
    std::fs::rename(&tmp_path, &path)
}

/// Remove `channel`'s record if it is this process's (`pid`). Another process
/// on the same channel name owns its own record; this one leaves it alone.
pub(super) fn remove_own_record(dir: &Path, channel: &str, pid: u32) {
    let path = dir.join(record_file_name(channel));
    if read_record(&path).is_some_and(|r| r.pid == pid) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Read one record, refusing an oversized file or out-of-bounds fields.
fn read_record(path: &Path) -> Option<InstanceRecord> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let record: InstanceRecord = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let in_bounds = !record.channel.is_empty()
        && record.channel.len() <= MAX_CHANNEL_LEN
        && !record.lan_key.is_empty()
        && record.lan_key.len() <= MAX_KEY_LEN
        && record.version.len() <= MAX_VERSION_LEN
        && record.port != 0;
    in_bounds.then_some(record)
}

/// The other LAN-enabled instances on this host: records in `dir` that are
/// fresh (rewritten within [`STALE_AFTER_MS`]), whose process is alive, and
/// that are not this instance (neither `own_channel` nor `own_pid`). Sorted by
/// channel, one per channel, at most [`MAX_SIBLINGS`].
pub(super) fn read_siblings(
    dir: &Path,
    own_channel: &str,
    own_pid: u32,
    now_ms: u64,
    pid_alive: impl Fn(u32) -> bool,
) -> Vec<InstanceRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<InstanceRecord> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .take(MAX_FILES_SCANNED)
        .filter_map(|p| read_record(&p))
        // A timestamp far in the future is as untrustworthy as an old one.
        .filter(|r| now_ms.abs_diff(r.updated_at_ms) <= STALE_AFTER_MS)
        .filter(|r| r.channel != own_channel && r.pid != own_pid)
        .collect();
    // Freshest first within a channel, so the dedup keeps the live writer if
    // two files claim the same channel.
    found.sort_by(|a, b| {
        a.channel
            .to_lowercase()
            .cmp(&b.channel.to_lowercase())
            .then_with(|| a.channel.cmp(&b.channel))
            .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
    });
    found.dedup_by(|later, earlier| later.channel == earlier.channel);
    // Last, because it is the one check that asks the OS.
    found.retain(|r| pid_alive(r.pid));
    found.truncate(MAX_SIBLINGS);
    found
}

/// A sibling as it appears in the UDP reply. `auth_key` is that sibling's
/// `lan_key`, under the field name every client already reads.
pub(super) fn sibling_json(record: &InstanceRecord) -> serde_json::Value {
    json!({
        "channel": record.channel,
        "port": record.port,
        "auth_key": record.lan_key,
        "version": record.version,
    })
}

/// Set `response["siblings"]` to as many of `siblings` as keep the serialized
/// reply within `budget` bytes. Always present, empty when there are none, so
/// a client can tell "no siblings" from "a build that does not send them".
pub(super) fn attach_siblings(
    response: &mut serde_json::Value,
    siblings: &[serde_json::Value],
    budget: usize,
) {
    response["siblings"] = json!([]);
    let mut size = serde_json::to_vec(response)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    let mut kept = Vec::new();
    for sibling in siblings {
        let extra = serde_json::to_vec(sibling)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        let separator = usize::from(!kept.is_empty());
        if size.saturating_add(extra).saturating_add(separator) > budget {
            break;
        }
        size += extra + separator;
        kept.push(sibling.clone());
    }
    response["siblings"] = serde_json::Value::Array(kept);
}

/// Where this instance's record goes: `None` when the shared root can't be
/// resolved, and always `None` under `cargo test`, whose `LanDiscovery::start`
/// calls must not overwrite a real instance's record on the developer's
/// machine. Tests that exercise the files pass a temp directory.
pub(super) fn instances_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::registry::resolve_shared_lan_instances_dir()
}

impl LanDiscovery {
    fn own_record(&self) -> InstanceRecord {
        InstanceRecord {
            channel: crate::backend::reactive::registry::local_channel_id(),
            port: self.port,
            lan_key: self.auth_key.clone(),
            version: self.version.clone(),
            pid: std::process::id(),
            updated_at_ms: agentmux_common::time::now_ms_u64(),
        }
    }

    /// Write (or refresh) this instance's record, unless it has been withdrawn.
    pub(super) fn publish_instance_record(&self) {
        let live = self.instance_record_live.lock();
        let (true, Some(dir)) = (*live, self.instances_dir.as_ref()) else {
            return;
        };
        if let Err(e) = write_record(dir, &self.own_record()) {
            tracing::debug!(error = %e, "LAN instance record not written");
        }
    }

    /// Remove this instance's record and stop it being written again. The
    /// flag is cleared under the same lock `publish_instance_record` holds, so
    /// a rewrite racing a stop cannot bring the file back. Idempotent.
    pub(super) fn withdraw_instance_record(&self) {
        let mut live = self.instance_record_live.lock();
        if !std::mem::replace(&mut *live, false) {
            return;
        }
        if let Some(dir) = self.instances_dir.as_ref() {
            let channel = crate::backend::reactive::registry::local_channel_id();
            remove_own_record(dir, &channel, std::process::id());
        }
    }

    /// Rewrites the record every [`REWRITE_INTERVAL`] until cancelled by
    /// `shutdown()`. Holds its own `Arc`, like the other loops `start()` spawns.
    pub(super) async fn instance_record_loop(self: Arc<Self>, mut cancel: oneshot::Receiver<()>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(REWRITE_INTERVAL) => self.publish_instance_record(),
                _ = &mut cancel => return,
            }
        }
    }

    /// The `siblings` of a probe reply, as JSON, read fresh on each probe.
    pub(super) fn sibling_entries(&self) -> Vec<serde_json::Value> {
        let Some(dir) = self.instances_dir.as_ref() else {
            return Vec::new();
        };
        read_siblings(
            dir,
            &crate::backend::reactive::registry::local_channel_id(),
            std::process::id(),
            agentmux_common::time::now_ms_u64(),
            crate::backend::reactive::registry::pid_alive,
        )
        .iter()
        .map(sibling_json)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn record(channel: &str, pid: u32, updated_at_ms: u64) -> InstanceRecord {
        InstanceRecord {
            channel: channel.to_string(),
            port: 29700,
            lan_key: format!("lan-key-{channel}"),
            version: "0.59.7".to_string(),
            pid,
            updated_at_ms,
        }
    }

    fn siblings(dir: &Path, alive: impl Fn(u32) -> bool) -> Vec<String> {
        read_siblings(dir, "self-channel", 1, NOW, alive)
            .into_iter()
            .map(|r| r.channel)
            .collect()
    }

    #[test]
    fn a_written_record_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let r = record("stable", 42, NOW);
        write_record(dir.path(), &r).unwrap();
        assert_eq!(
            read_siblings(dir.path(), "self-channel", 1, NOW, |_| true),
            vec![r]
        );
        // No temp file left behind.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(record_file_name("stable"))]);
    }

    #[cfg(unix)]
    #[test]
    fn a_record_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("stable", 42, NOW)).unwrap();
        let mode = std::fs::metadata(dir.path().join(record_file_name("stable")))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// `<stem>-<8 hex>.json`: the readable stem, then the hash tag.
    fn stem_of(name: &str) -> &str {
        let base = name.strip_suffix(".json").expect("ends in .json");
        let (stem, tag) = base.rsplit_once('-').expect("has a hash tag");
        assert_eq!(tag.len(), 8);
        assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
        stem
    }

    #[test]
    fn a_channel_cannot_name_a_path_outside_the_directory() {
        assert_eq!(stem_of(&record_file_name("../../etc/passwd")), "______etc_passwd");
        assert_eq!(stem_of(&record_file_name("C:\\Windows\\x")), "c__windows_x");
        assert_eq!(stem_of(&record_file_name("local-main-b28b7a")), "local-main-b28b7a");
        assert_eq!(stem_of(&record_file_name("")), "_");
        let long = "x".repeat(500);
        assert_eq!(stem_of(&record_file_name(&long)).len(), MAX_CHANNEL_LEN);
        for name in ["../../etc/passwd", "C:\\Windows\\x", "a/b", ""] {
            let f = record_file_name(name);
            assert!(!f.contains('/') && !f.contains('\\') && !f.contains(".."), "{f}");
        }

        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("instances");
        write_record(&nested, &record("../escape", 42, NOW)).unwrap();
        assert!(nested.join(record_file_name("../escape")).exists());
        let outside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(outside, vec![std::ffi::OsString::from("instances")]);
    }

    #[test]
    fn channels_with_the_same_slug_get_their_own_files() {
        // The slug alone is lossy (#4297).
        for (a, b) in [("Foo", "foo"), ("foo bar", "foo_bar"), ("dev/x", "dev_x")] {
            assert_eq!(stem_of(&record_file_name(a)), stem_of(&record_file_name(b)));
            assert_ne!(record_file_name(a), record_file_name(b), "{a} vs {b}");
        }
        assert_eq!(record_file_name("stable"), record_file_name("stable"), "stable per name");

        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("foo bar", 50, NOW)).unwrap();
        write_record(dir.path(), &record("foo_bar", 51, NOW)).unwrap();
        assert_eq!(siblings(dir.path(), |_| true), vec!["foo bar", "foo_bar"]);
    }

    #[test]
    fn stale_dead_and_own_records_are_not_siblings() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("fresh", 10, NOW - 5_000)).unwrap();
        write_record(dir.path(), &record("stale", 11, NOW - STALE_AFTER_MS - 1)).unwrap();
        write_record(dir.path(), &record("future", 12, NOW + STALE_AFTER_MS + 1)).unwrap();
        write_record(dir.path(), &record("dead", 13, NOW)).unwrap();
        write_record(dir.path(), &record("self-channel", 14, NOW)).unwrap();
        write_record(dir.path(), &record("same-pid", 1, NOW)).unwrap();
        assert_eq!(siblings(dir.path(), |pid| pid != 13), vec!["fresh"]);
    }

    #[test]
    fn siblings_are_sorted_bounded_and_one_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_SIBLINGS + 4) {
            write_record(
                dir.path(),
                &record(&format!("ch-{i:02}"), 100 + i as u32, NOW),
            )
            .unwrap();
        }
        let got = siblings(dir.path(), |_| true);
        assert_eq!(got.len(), MAX_SIBLINGS);
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted);

        // Two files claiming one channel: the fresher is kept.
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("dup", 20, NOW - 10_000)).unwrap();
        let mut other = record("dup", 21, NOW);
        other.port = 29704;
        std::fs::write(
            dir.path().join("other.json"),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        let got = read_siblings(dir.path(), "self-channel", 1, NOW, |_| true);
        assert_eq!(got, vec![other]);
    }

    #[test]
    fn malformed_oversized_and_out_of_bounds_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("junk.json"), b"not json").unwrap();
        std::fs::write(
            dir.path().join("big.json"),
            vec![b' '; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(dir.path().join("ignored.txt"), b"{}").unwrap();
        let mut no_key = record("nokey", 30, NOW);
        no_key.lan_key.clear();
        let mut zero_port = record("zeroport", 31, NOW);
        zero_port.port = 0;
        let long_channel = record(&"c".repeat(MAX_CHANNEL_LEN + 1), 32, NOW);
        for (name, r) in [
            ("nokey", no_key),
            ("zeroport", zero_port),
            ("long", long_channel),
        ] {
            std::fs::write(
                dir.path().join(format!("{name}.json")),
                serde_json::to_vec(&r).unwrap(),
            )
            .unwrap();
        }
        write_record(dir.path(), &record("good", 33, NOW)).unwrap();
        assert_eq!(siblings(dir.path(), |_| true), vec!["good"]);
    }

    #[test]
    fn only_the_owning_process_removes_a_record() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("stable", 42, NOW)).unwrap();
        remove_own_record(dir.path(), "stable", 43);
        assert!(
            dir.path().join(record_file_name("stable")).exists(),
            "another pid's record stays"
        );
        remove_own_record(dir.path(), "stable", 42);
        assert!(!dir.path().join(record_file_name("stable")).exists());
    }

    #[test]
    fn a_sibling_carries_its_lan_key_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        // A record file with an extra field a forger (or a future build) added.
        let mut raw = serde_json::to_value(record("stable", 42, NOW)).unwrap();
        raw["auth_key"] = json!("FULL-AUTH-KEY-MUST-NOT-LEAK");
        std::fs::write(
            dir.path().join("stable.json"),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();
        let got = read_siblings(dir.path(), "self-channel", 1, NOW, |_| true);
        let entry = sibling_json(&got[0]);
        assert_eq!(
            entry,
            json!({"channel": "stable", "port": 29700, "auth_key": "lan-key-stable", "version": "0.59.7"})
        );
        assert!(!entry.to_string().contains("FULL-AUTH-KEY"));
    }

    #[test]
    fn siblings_stop_at_the_byte_budget() {
        let mut response = probe_response_json("v0.59.7", "narko", "0.59.7", 29704, "k");
        let many: Vec<_> = (0..MAX_SIBLINGS)
            .map(|i| sibling_json(&record(&format!("local-main-channel-{i:02}"), 1, NOW)))
            .collect();
        attach_siblings(&mut response, &many, MAX_REPLY_BYTES);
        let size = serde_json::to_vec(&response).unwrap().len();
        let kept = response["siblings"].as_array().unwrap().len();
        assert!(size <= MAX_REPLY_BYTES, "reply is {size} bytes");
        assert!(kept > 0 && kept < MAX_SIBLINGS, "kept {kept}");

        let mut response = probe_response_json("v0.59.7", "narko", "0.59.7", 29704, "k");
        attach_siblings(&mut response, &[], MAX_REPLY_BYTES);
        assert_eq!(response["siblings"], json!([]));
    }

    // -- On a `LanDiscovery` (idle mDNS daemon, none of `start()`'s tasks) --

    fn discovery(dir: Option<PathBuf>) -> Arc<LanDiscovery> {
        Arc::new(LanDiscovery {
            daemon: mdns_sd::ServiceDaemon::new()
                .expect("daemon construction (no register/browse)"),
            instances: Arc::new(RwLock::new(HashMap::new())),
            instance_id: "v0.59.7".to_string(),
            event_bus: Arc::new(crate::backend::eventbus::EventBus::new()),
            service_fullname: String::new(),
            auth_key: "own-lan-key".to_string(),
            hostname: "narko".to_string(),
            version: "0.59.7".to_string(),
            port: 29704,
            udp_cancel: Mutex::new(None),
            udp_peer_cancel: Mutex::new(None),
            agent_names_cancel: Mutex::new(None),
            instance_record_cancel: Mutex::new(None),
            instances_dir: dir,
            instance_record_live: Mutex::new(true),
            announced_v4: Arc::new(Mutex::new(BTreeSet::new())),
            monitored: true,
            started_at: std::time::Instant::now(),
        })
    }

    /// A pid that is alive for the whole test and is not ours: our parent
    /// (the test harness or cargo).
    fn another_live_pid() -> u32 {
        let me = sysinfo::Pid::from_u32(std::process::id());
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[me]), true);
        let parent = sys
            .process(me)
            .and_then(|p| p.parent())
            .expect("test process has a parent");
        assert!(crate::backend::reactive::registry::pid_alive(
            parent.as_u32()
        ));
        parent.as_u32()
    }

    /// The channel is read from the process environment, which other tests
    /// change under this lock.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::ISOLATED_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn an_instance_publishes_and_withdraws_its_record() {
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let d = discovery(Some(dir.path().to_path_buf()));
        let path = dir.path().join(record_file_name(
            &crate::backend::reactive::registry::local_channel_id(),
        ));

        d.publish_instance_record();
        let r = read_record(&path).expect("written on publish");
        assert_eq!(
            (r.port, r.lan_key.as_str(), r.pid),
            (29704, "own-lan-key", std::process::id())
        );

        d.withdraw_instance_record();
        assert!(!path.exists(), "removed on withdraw");
        d.publish_instance_record();
        assert!(
            !path.exists(),
            "a rewrite after withdrawal does not bring it back"
        );
    }

    #[test]
    fn shutdown_withdraws_the_record() {
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let d = discovery(Some(dir.path().to_path_buf()));
        d.publish_instance_record();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        d.shutdown();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_probe_reply_lists_live_siblings_but_the_desktop_reply_does_not() {
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let now = agentmux_common::time::now_ms_u64();
        write_record(
            dir.path(),
            &record("sibling-channel", another_live_pid(), now),
        )
        .unwrap();
        write_record(dir.path(), &record("crashed-channel", u32::MAX, now)).unwrap();
        let d = discovery(Some(dir.path().to_path_buf()));
        d.publish_instance_record(); // this instance's own record is never listed

        let reply = d.build_probe_response();
        assert_eq!(
            reply["siblings"],
            json!([{
                "channel": "sibling-channel",
                "port": 29700,
                "auth_key": "lan-key-sibling-channel",
                "version": "0.59.7",
            }])
        );
        assert_eq!(reply["auth_key"], "own-lan-key");
        assert_eq!(reply["type"], UDP_RESPONSE_TYPE);
        assert!(d.build_identity_response().get("siblings").is_none());
    }

    #[test]
    fn the_reply_carries_install_id_and_channel_count_and_stays_one_datagram() {
        let _env = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let now = agentmux_common::time::now_ms_u64();
        let pid = another_live_pid();
        // As many siblings as a reply lists, each with the longest channel
        // name a record may carry, so the byte budget is what limits them.
        for i in 0..MAX_SIBLINGS {
            let channel = format!("{i:02}{}", "c".repeat(MAX_CHANNEL_LEN - 2));
            let mut r = record(&channel, pid, now);
            r.lan_key = "k".repeat(64);
            write_record(dir.path(), &r).unwrap();
        }
        let d = discovery(Some(dir.path().to_path_buf()));
        let install_id = "mw3am46w5weex4a4fqrc3avnua";

        let reply = d.probe_response_with(Some(install_id), Some(99));
        let size = serde_json::to_vec(&reply).unwrap().len();
        assert!(size <= MAX_REPLY_BYTES, "reply is {size} bytes");
        assert_eq!(reply["install_id"], install_id);
        assert_eq!(reply["channels_running"], 99);
        let kept = reply["siblings"].as_array().unwrap().len();
        assert!(kept > 0 && kept < MAX_SIBLINGS, "siblings are trimmed first: kept {kept}");

        let identity = d.identity_response_with(Some(install_id), Some(3));
        assert_eq!(identity["install_id"], install_id);
        assert_eq!(identity["channels_running"], 3);

        // Unknown: the fields are left out, not sent empty.
        let bare = d.identity_response_with(None, None);
        assert!(bare.get("install_id").is_none());
        assert!(bare.get("channels_running").is_none());
    }

    fn probe_bytes() -> Vec<u8> {
        serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE, "v": UDP_PROTOCOL_VERSION})).unwrap()
    }

    #[tokio::test]
    async fn the_responder_takes_over_the_port_once_its_holder_lets_go() {
        let holder = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let d = discovery(None);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let task = tokio::spawn(d.clone().udp_responder_loop_on(
            port,
            std::time::Duration::from_millis(50),
            cancel_rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(!task.is_finished(), "a failed bind keeps retrying");
        drop(holder);

        let prober = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut buf = [0u8; 2048];
        let reply = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let _ = prober.send_to(&probe_bytes(), ("127.0.0.1", port)).await;
                let wait = std::time::Duration::from_millis(200);
                if let Ok(Ok((len, _))) =
                    tokio::time::timeout(wait, prober.recv_from(&mut buf)).await
                {
                    break serde_json::from_slice::<serde_json::Value>(&buf[..len]).unwrap();
                }
            }
        })
        .await
        .expect("the responder answers once it holds the port");
        assert_eq!(reply["type"], UDP_RESPONSE_TYPE);
        assert_eq!(reply["siblings"], json!([]));

        cancel_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn the_responder_stops_while_still_waiting_for_the_port() {
        let holder = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let task = tokio::spawn(discovery(None).udp_responder_loop_on(
            port,
            std::time::Duration::from_secs(3600),
            cancel_rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("cancel ends the retry wait")
            .unwrap();
        drop(holder);
    }
}
