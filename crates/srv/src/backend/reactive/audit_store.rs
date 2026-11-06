// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The durable copy of the jekt audit trail. The handler keeps the last
//! [`AUDIT_LOG_MAX`](super::AUDIT_LOG_MAX) entries in memory for
//! `GET /agentmux/reactive/audit`, and loses them at every restart. Every entry
//! is also appended here, as one JSON line, to a file that is never rotated or
//! truncated: AgentMux keeps agent history, and the record of what agents did,
//! forever.
//!
//! Entries hold a hash and the length of each message, never its text, plus the
//! sender, target, outcome and a caller's free-text note. The file is shared by
//! every srv instance on the machine, so each line names its channel.
//!
//! The handler records while holding its lock, so recording does no I/O: it
//! queues the line for a writer thread.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::thread::JoinHandle;

use super::types::AuditLogEntry;

static SINK: OnceLock<Sender<AuditLogEntry>> = OnceLock::new();

/// `<shared root>/audit/jekt-audit.jsonl`, or `None` while this srv has given the shared folder up.
pub fn default_path() -> Option<PathBuf> {
    crate::registry::resolve_global_shared_root().map(|root| root.join("audit").join("jekt-audit.jsonl"))
}

/// The shared file to read, whether or not this srv may write to it: a srv that gave the folder up
/// still lists what the account's other srvs recorded.
pub fn shared_path_for_reading() -> Option<PathBuf> {
    crate::registry::shared_root_ignoring_deferral().map(|root| root.join("audit").join("jekt-audit.jsonl"))
}

/// `<this channel's folder>/audit/jekt-audit.jsonl`: where a srv that gave the shared folder up keeps its own
/// rows. A line goes to exactly one of this and the shared file. It is the channel's own folder, not the
/// per-version data folder (`--wavedata`, `AGENTMUX_DATA_DIR`): a srv that deferred is usually upgraded next, and
/// the new version must still find the rows the old one kept.
pub fn own_path() -> Option<PathBuf> {
    std::env::var_os("AGENTMUX_INSTANCE_DIR").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("audit").join("jekt-audit.jsonl"))
}

/// Where this srv appends: the shared file, or its own when it doesn't use the shared folder.
pub fn write_path() -> Option<PathBuf> {
    default_path().or_else(own_path)
}

/// Start recording where this srv writes ([`write_path`]), and say so in the log when that is its own file.
pub fn install_for_this_srv() {
    match write_path() {
        Some(path) => {
            if default_path().is_none() {
                tracing::info!(path = %path.display(), "jekt audit: the shared folder is not used; this srv keeps its rows in its own file");
            }
            let channel_name = std::env::var("AGENTMUX_INSTANCE_CHANNEL").unwrap_or_default();
            if let Err(e) = install(path.clone(), channel_name) {
                tracing::warn!(path = %path.display(), error = %e, "jekt audit: not kept in a file");
            }
        }
        None => tracing::warn!("jekt audit: no shared directory and no channel directory; not kept in a file"),
    }
    // The index of what agents do goes beside it.
    crate::backend::audit_activity::install_for_this_srv();
}

/// Start recording to `path` for the rest of this process. The first call wins.
pub fn install(path: PathBuf, channel_name: String) -> std::io::Result<()> {
    let (tx, _writer) = spawn_writer(path, channel_name)?;
    let _ = SINK.set(tx);
    Ok(())
}

/// Queue `entry` for the audit file. Nothing happens before [`install`].
pub fn record(entry: &AuditLogEntry) {
    if let Some(tx) = SINK.get() {
        let _ = tx.send(entry.clone());
    }
}

/// Add `entry` to the in-memory `ring` that serves `GET /agentmux/reactive/audit`
/// (the last `max` entries), and queue it for the audit file, which keeps every one.
pub(super) fn push(ring: &mut Vec<AuditLogEntry>, max: usize, entry: AuditLogEntry) {
    record(&entry);
    if ring.len() >= max {
        ring.remove(0);
    }
    ring.push(entry);
}

/// Open `path` for appending and start the thread that writes what is sent.
/// The thread ends when the sender is dropped.
pub(super) fn spawn_writer(
    path: PathBuf,
    channel_name: String,
) -> std::io::Result<(Sender<AuditLogEntry>, JoinHandle<()>)> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let (tx, rx) = channel::<AuditLogEntry>();
    let writer = std::thread::Builder::new().name("jekt-audit-writer".into()).spawn(move || {
        for entry in rx {
            if let Err(e) = write_line(&mut file, &channel_name, &entry) {
                tracing::warn!(path = %path.display(), error = %e, "jekt audit: could not append an entry");
            }
        }
    })?;
    Ok((tx, writer))
}

fn write_line(file: &mut std::fs::File, channel_name: &str, entry: &AuditLogEntry) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&stamped(entry, channel_name)).map_err(std::io::Error::other)?;
    line.push(b'\n');
    // One write call per line, so lines from several instances don't interleave.
    file.write_all(&line)?;
    file.flush()
}

/// `entry` with where it is written from: this computer, the channel and the srv's version. Whatever the entry said
/// of them is replaced: a row is written by the srv that recorded it.
fn stamped(entry: &AuditLogEntry, channel_name: &str) -> AuditLogEntry {
    static HOST: OnceLock<String> = OnceLock::new();
    AuditLogEntry {
        host: Some(HOST.get_or_init(super::registry::local_host_label).clone()),
        channel: Some(channel_name.to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        ..entry.clone()
    }
}

/// The entries in the file at `path`, oldest first; lines that don't parse are skipped.
#[allow(dead_code)] // used by the tests; no production reader yet
pub fn read_all(path: &Path) -> Vec<AuditLogEntry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(target: &str, n: usize) -> AuditLogEntry {
        AuditLogEntry {
            timestamp: 1_000 + n as u64,
            source_agent: Some("Korp".into()),
            target_agent: target.into(),
            block_id: "b1".into(),
            message_hash: format!("{n:016x}"),
            message_length: n,
            success: true,
            error_message: None,
            request_id: format!("r{n}"),
            outcome: None,
            reason: None,
            event_kind: "delivery".into(),
            evicted_block: None,
            evicted_agent: None,
            audit_source_uid: String::new(),
            lan_claimed_uid: String::new(),
            wan: None,
            cli_native: None,
            turn: None,
            host: None,
            channel: None,
            version: None,
            tier: None,
            trust: None,
            msg_id: None,
        }
    }

    #[test]
    fn a_srv_without_the_shared_folder_keeps_its_rows_in_its_channel_folder_not_the_per_version_data_folder() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let saved = (std::env::var_os("AGENTMUX_INSTANCE_DIR"), std::env::var_os("AGENTMUX_DATA_DIR"));
        // The data folder is `<channel folder>/versions/<version>/data`: the next version does not see it.
        std::env::set_var("AGENTMUX_INSTANCE_DIR", dir.path());
        std::env::set_var("AGENTMUX_DATA_DIR", dir.path().join("versions").join("1.2.3").join("data"));
        let own = own_path();
        for (name, value) in [("AGENTMUX_INSTANCE_DIR", saved.0), ("AGENTMUX_DATA_DIR", saved.1)] {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        assert_eq!(own, Some(dir.path().join("audit").join("jekt-audit.jsonl")));
    }

    #[test]
    fn every_entry_is_appended_with_its_channel_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit").join("jekt-audit.jsonl");

        // More than the in-memory ring keeps.
        let (tx, writer) = spawn_writer(path.clone(), "dev-a".into()).unwrap();
        for n in 0..250 {
            tx.send(entry("Lark", n)).unwrap();
        }
        drop(tx);
        writer.join().unwrap();

        // A second process appends to the same file.
        let (tx, writer) = spawn_writer(path.clone(), "dev-b".into()).unwrap();
        tx.send(entry("Lark", 250)).unwrap();
        drop(tx);
        writer.join().unwrap();

        let entries = read_all(&path);
        assert_eq!(entries.len(), 251, "nothing is dropped, and nothing is truncated on reopen");
        assert_eq!(entries[0].message_length, 0);
        assert_eq!(entries[250].message_length, 250);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.lines().next().unwrap().contains("\"channel\":\"dev-a\""));
        assert!(text.lines().last().unwrap().contains("\"channel\":\"dev-b\""));
        assert!(!text.contains("hello"), "message text is never written");
    }

    #[test]
    fn a_line_that_does_not_parse_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jekt-audit.jsonl");
        let (tx, writer) = spawn_writer(path.clone(), "c".into()).unwrap();
        tx.send(entry("Lark", 1)).unwrap();
        drop(tx);
        writer.join().unwrap();
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{torn\n").unwrap();
        assert_eq!(read_all(&path).len(), 1);
    }
}
