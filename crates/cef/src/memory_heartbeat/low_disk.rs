// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Low disk space on the drive that holds AgentMux's data, shown with the
//! other health signals in the backend status panel.
//!
//! srv keeps every transcript in SQLite stores behind one write connection.
//! When the drive is nearly full its writes slow down, and a write that
//! stalls holds that connection for seconds, so sends and agents' saves wait
//! behind it (issue #5141). Long holds were seen while the system drive was
//! nearly full, and stopped after space was freed. A drive that is filling up
//! is cheap to warn about before it gets there.

/// Warn below either line, so a small drive warns on gigabytes and a large
/// one on its share.
pub const WARN_PCT: f64 = 8.0;
pub const WARN_GB: f64 = 10.0;
/// Critical below either line.
pub const CRITICAL_PCT: f64 = 4.0;
pub const CRITICAL_GB: f64 = 3.0;
/// Leaving a level needs this much headroom over its line, so a drive
/// hovering at the line doesn't make the signal flicker.
const CLEAR_PCT: f64 = 1.0;
const CLEAR_GB: f64 = 2.0;

const BYTES_PER_GB: f64 = 1_073_741_824.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskLevel {
    Normal,
    Warn,
    Critical,
}

impl DiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            DiskLevel::Normal => "normal",
            DiskLevel::Warn => "warn",
            DiskLevel::Critical => "critical",
        }
    }
}

/// One reading of the drive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiskReading {
    pub free_gb: f64,
    pub total_gb: f64,
    pub free_pct: f64,
}

impl DiskReading {
    pub fn from_bytes(free: u64, total: u64) -> Self {
        let free_gb = free as f64 / BYTES_PER_GB;
        let total_gb = total as f64 / BYTES_PER_GB;
        let free_pct = if total > 0 { free as f64 / total as f64 * 100.0 } else { 100.0 };
        DiskReading { free_gb, total_gb, free_pct }
    }
}

fn below(r: &DiskReading, pct: f64, gb: f64) -> bool {
    r.free_pct < pct || r.free_gb < gb
}

/// The level for `r`, given the level before. Entering a level takes being
/// under either of its lines; leaving it takes being clear of both by the
/// margin.
pub fn classify(r: &DiskReading, prev: DiskLevel) -> DiskLevel {
    let critical = if prev == DiskLevel::Critical {
        below(r, CRITICAL_PCT + CLEAR_PCT, CRITICAL_GB + CLEAR_GB)
    } else {
        below(r, CRITICAL_PCT, CRITICAL_GB)
    };
    if critical {
        return DiskLevel::Critical;
    }
    let warn = if prev == DiskLevel::Normal {
        below(r, WARN_PCT, WARN_GB)
    } else {
        below(r, WARN_PCT + CLEAR_PCT, WARN_GB + CLEAR_GB)
    };
    if warn {
        DiskLevel::Warn
    } else {
        DiskLevel::Normal
    }
}

/// What the heartbeat does with each reading.
#[derive(Debug)]
pub struct LowDiskTracker {
    level: DiskLevel,
}

impl Default for LowDiskTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LowDiskTracker {
    pub fn new() -> Self {
        LowDiskTracker { level: DiskLevel::Normal }
    }

    /// Fold in a reading. Returns the level to push: on every change, and on
    /// every reading while not normal, so the panel's numbers stay current.
    /// A steady normal pushes nothing.
    pub fn observe(&mut self, r: &DiskReading) -> Option<DiskLevel> {
        let next = classify(r, self.level);
        let changed = next != self.level;
        self.level = next;
        (changed || next != DiskLevel::Normal).then_some(next)
    }
}

/// The payload of a `health-signal` event of kind `disk`.
pub fn payload(level: DiskLevel, r: &DiskReading, path: &str) -> serde_json::Value {
    if level == DiskLevel::Normal {
        return serde_json::json!({ "kind": "disk", "level": "normal" });
    }
    serde_json::json!({
        "kind": "disk",
        "level": level.as_str(),
        "disk_free_gb": (r.free_gb * 10.0).round() / 10.0,
        "disk_total_gb": r.total_gb.round(),
        "disk_free_pct": (r.free_pct * 10.0).round() / 10.0,
        "disk_path": path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gb(free_gb: f64, total_gb: f64) -> DiskReading {
        DiskReading { free_gb, total_gb, free_pct: free_gb / total_gb * 100.0 }
    }

    #[test]
    fn a_large_drive_warns_on_its_share() {
        // 3.7 TB: 6% free is still 222 GB, but the share is what matters.
        assert_eq!(classify(&gb(400.0, 3700.0), DiskLevel::Normal), DiskLevel::Normal);
        assert_eq!(classify(&gb(222.0, 3700.0), DiskLevel::Normal), DiskLevel::Warn);
        assert_eq!(classify(&gb(140.0, 3700.0), DiskLevel::Normal), DiskLevel::Critical);
    }

    #[test]
    fn a_small_drive_warns_on_gigabytes() {
        // 64 GB at 14% free is fine by share, but 9 GB is little room.
        assert_eq!(classify(&gb(9.0, 64.0), DiskLevel::Normal), DiskLevel::Warn);
        assert_eq!(classify(&gb(2.5, 64.0), DiskLevel::Normal), DiskLevel::Critical);
        assert_eq!(classify(&gb(20.0, 64.0), DiskLevel::Normal), DiskLevel::Normal);
    }

    #[test]
    fn leaving_a_level_needs_headroom() {
        let mut t = LowDiskTracker::new();
        assert_eq!(t.observe(&gb(290.0, 3700.0)), Some(DiskLevel::Warn)); // 7.8%
        // 8.3%: over the line, inside the margin, so still a warning.
        assert_eq!(t.observe(&gb(307.0, 3700.0)), Some(DiskLevel::Warn));
        // 9.2%: clear of it, so it ends, once.
        assert_eq!(t.observe(&gb(340.0, 3700.0)), Some(DiskLevel::Normal));
        assert_eq!(t.observe(&gb(340.0, 3700.0)), None, "a steady normal is silent");
    }

    #[test]
    fn critical_steps_down_through_warn() {
        let mut t = LowDiskTracker::new();
        assert_eq!(t.observe(&gb(100.0, 3700.0)), Some(DiskLevel::Critical)); // 2.7%
        assert_eq!(t.observe(&gb(160.0, 3700.0)), Some(DiskLevel::Critical)); // 4.3%: in the margin
        assert_eq!(t.observe(&gb(200.0, 3700.0)), Some(DiskLevel::Warn)); // 5.4%
    }

    #[test]
    fn the_payload_carries_the_numbers_only_while_not_normal() {
        let r = gb(222.0, 3700.0);
        let p = payload(DiskLevel::Warn, &r, "C:\\Users\\me\\.agentmux");
        assert_eq!(p["kind"], "disk");
        assert_eq!(p["level"], "warn");
        assert_eq!(p["disk_free_gb"], 222.0);
        assert_eq!(p["disk_free_pct"], 6.0);
        assert_eq!(p["disk_path"], "C:\\Users\\me\\.agentmux");
        let n = payload(DiskLevel::Normal, &r, "x");
        assert!(n.get("disk_free_gb").is_none());
    }

    #[test]
    fn a_reading_from_bytes() {
        let r = DiskReading::from_bytes(50 * 1_073_741_824, 1000 * 1_073_741_824);
        assert!((r.free_gb - 50.0).abs() < 1e-9 && (r.free_pct - 5.0).abs() < 1e-9);
    }
}
