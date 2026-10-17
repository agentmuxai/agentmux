// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The incident record: what went wrong and what AgentMux did about it, kept
//! for the user (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md
//! §4.1). The host writes the cases it handles (`agentmux-cef`
//! `commands::incidents`), the launcher the ones only it sees (a restart);
//! the frontend shows each one the user hasn't closed.
//!
//! An append-only JSON-lines file in the instance's data folder: an incident
//! is one line, closing its notice appends an acknowledgement line. Written
//! synchronously, since the writers are recovery paths that may be about to
//! exit; incidents are rare, and each write is one small append.

use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file's name in the data folder.
pub const FILE_NAME: &str = "incidents.jsonl";

/// The file keeps about this many lines; past twice that it is cut back.
const KEEP_LINES: usize = 200;

/// One incident, as the user is told about it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub id: String,
    pub at_ms: u64,
    /// A closed set (spec §4.4): `renderer_crashed`, `renderer_oom`,
    /// `renderer_hung`, `quit_no_window`, `host_restarted`,
    /// `host_restarted_safe_mode`, `srv_restarted`, `memory_give_up`,
    /// `host_give_up`.
    pub kind: String,
    /// What happened, in plain words.
    pub what: String,
    /// Why, when it is known; never guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// What AgentMux did that keeps the user's work: only what this path did.
    #[serde(default)]
    pub did: Vec<String>,
    /// What may not have survived, when this path knows.
    #[serde(default)]
    pub lost: Vec<String>,
    /// Which log has the details, and when, for a bug report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Set by `read` from the acknowledgement lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at_ms: Option<u64>,
}

impl Incident {
    /// A new incident now; `log` names the log with the details ("host",
    /// "launcher").
    pub fn new(kind: &str, what: impl Into<String>, log: &str) -> Self {
        let at_ms = crate::time::now_ms_u64();
        // Unique enough for a handful of records: the time, and a hash seeded
        // per process at random.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(at_ms);
        h.write_u32(std::process::id());
        Incident {
            id: format!("{at_ms}-{:08x}", h.finish() as u32),
            at_ms,
            kind: kind.to_string(),
            what: what.into(),
            cause: None,
            did: Vec::new(),
            lost: Vec::new(),
            evidence: Some(format!("{log} log, {}", iso_utc(at_ms))),
            acknowledged_at_ms: None,
        }
    }
}

/// One line of the file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Line {
    Ack { ack: String, at_ms: u64 },
    Incident(Incident),
}

/// Append `incident` to the file at `path`.
pub fn append(path: &Path, incident: &Incident) -> std::io::Result<()> {
    write_line(path, &Line::Incident(incident.clone()))
}

/// Append the acknowledgement of incident `id`.
pub fn append_ack(path: &Path, id: &str, at_ms: u64) -> std::io::Result<()> {
    write_line(path, &Line::Ack { ack: id.to_string(), at_ms })
}

/// The newest `max` incidents at `path`, newest first, each with its
/// acknowledgement. A missing file is no incidents.
pub fn read(path: &Path, max: usize) -> Vec<Incident> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut incidents: Vec<Incident> = Vec::new();
    let mut acks: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    // A torn last line (a crash mid-append) costs only itself.
    for line in text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()) {
        match line {
            Line::Ack { ack, at_ms } => {
                acks.insert(ack, at_ms);
            }
            Line::Incident(i) => incidents.push(i),
        }
    }
    for i in &mut incidents {
        i.acknowledged_at_ms = acks.get(&i.id).copied();
    }
    incidents.reverse();
    incidents.truncate(max);
    incidents
}

fn write_line(path: &Path, line: &Line) -> std::io::Result<()> {
    let json = serde_json::to_string(line).map_err(std::io::Error::other)?;
    std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| writeln!(f, "{json}"))?;
    compact(path);
    Ok(())
}

/// Keep the last `KEEP_LINES` lines once the file is past twice that.
fn compact(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= KEEP_LINES * 2 {
        return;
    }
    let kept = lines[lines.len() - KEEP_LINES..].join("\n") + "\n";
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, kept).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// `ms` since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incidents_read_back_newest_first_with_their_acknowledgements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut a = Incident::new("renderer_crashed", "A window crashed and was reloaded", "host");
        a.did = vec!["Every agent kept running".into()];
        let b = Incident::new("host_restarted", "AgentMux restarted", "launcher");
        append(&path, &a).unwrap();
        append(&path, &b).unwrap();
        append_ack(&path, &a.id, 99).unwrap();
        // A torn last line costs only itself.
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"id\":\"torn").unwrap();
        let got = read(&path, 50);
        assert_eq!(got.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), vec!["host_restarted", "renderer_crashed"]);
        assert_eq!(got[1].acknowledged_at_ms, Some(99));
        assert_eq!(got[0].acknowledged_at_ms, None);
        assert_eq!(got[1].did, vec!["Every agent kept running".to_string()]);
        assert_ne!(a.id, b.id);
        assert!(b.evidence.as_deref().unwrap().starts_with("launcher log, "));
        assert!(read(&dir.path().join("none.jsonl"), 50).is_empty());
    }

    #[test]
    fn the_file_is_cut_back_to_its_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        for n in 0..(KEEP_LINES * 2 + 1) {
            append(&path, &Incident::new("renderer_crashed", format!("n{n}"), "host")).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), KEEP_LINES);
        assert_eq!(read(&path, 1).first().map(|i| i.what.as_str()), Some(format!("n{}", KEEP_LINES * 2).as_str()));
    }

    #[test]
    fn times_are_written_as_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_792_186_299_395), "2026-10-16T21:31:39Z");
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00Z");
    }
}
