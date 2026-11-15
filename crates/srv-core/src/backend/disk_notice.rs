// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When the low-disk note goes out and what it says (`build_cache_service::disk_check` sends it).
//! Pure functions, so the decisions are tested without a disk, a clock or a running agent.
//!
//! The first version went to the owners of the three largest workspaces whatever their size, once
//! an hour for as long as the free space hovered at the threshold, as a message that starts a
//! turn. On one host an agent with a 3.5 GB folder was woken eight times in three hours, and the
//! text set "43 GB free" next to "below the 20 GB / 10% level". Now:
//!
//! - a note goes out again only after free space falls a further [`MORE_DROP`], or the level gets
//!   worse (warning to critical), and at most once an hour: never on a timer alone;
//! - the disk counts as recovered only with room to spare ([`recovered`]), so free space
//!   bouncing across the threshold is one event, not many;
//! - the text says which rule tripped and which did not;
//! - a warning is held for the agent's next turn; only a critical level starts one.

use std::path::Path;

use crate::backend::build_cache::{human, Policy};

/// How low the disk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    /// Under the warning level: free space below `warn_free_bytes` or `warn_free_pct`.
    Warning,
    /// Under half of the warning level on either rule.
    Critical,
}

/// Free space must fall this much below what the last note reported to be worth another note, and
/// rise this much above the warning level to count as recovered.
pub const MORE_DROP: u64 = 5 << 30;
/// Percentage points above the warning percentage that count as recovered.
pub const RECOVER_PCT: f64 = 1.0;
/// A note goes out at most this often (seconds), unless the level got worse.
pub const MIN_GAP_SECS: u64 = 3600;
/// A workspace whose build folders total this much hears of the warning.
pub const BIG_WORKSPACE: u64 = 10 << 30;
/// Or whose incremental caches total this much.
pub const BIG_INCREMENTAL: u64 = 2 << 30;

fn pct_free(free: u64, total: u64) -> f64 {
    if total > 0 {
        free as f64 * 100.0 / total as f64
    } else {
        0.0
    }
}

/// The level of `free` of `total` bytes under `policy`.
pub fn level(policy: &Policy, free: u64, total: u64) -> Level {
    if total == 0 {
        return Level::Ok;
    }
    if free < policy.warn_free_bytes / 2 || pct_free(free, total) < policy.warn_free_pct / 2.0 {
        Level::Critical
    } else if policy.is_low(free, total) {
        Level::Warning
    } else {
        Level::Ok
    }
}

/// Whether the disk is back above the warning level with room to spare, so the next drop is a new
/// event. Right at the threshold it is not: free space that bounces across it stays one event.
pub fn recovered(policy: &Policy, free: u64, total: u64) -> bool {
    total > 0 && free >= policy.warn_free_bytes.saturating_add(MORE_DROP) && pct_free(free, total) >= policy.warn_free_pct + RECOVER_PCT
}

/// The last note that went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub level: Level,
    /// Free bytes it reported.
    pub free: u64,
    /// Unix seconds.
    pub at: u64,
}

impl Sent {
    fn render(&self) -> String {
        let word = if self.level == Level::Critical { "critical" } else { "warning" };
        format!("{word} {} {}", self.free, self.at)
    }

    fn parse(text: &str) -> Option<Sent> {
        let mut it = text.split_whitespace();
        let level = match it.next()? {
            "warning" => Level::Warning,
            "critical" => Level::Critical,
            _ => return None,
        };
        Some(Sent { level, free: it.next()?.parse().ok()?, at: it.next()?.parse().ok()? })
    }
}

/// The record of the last note, shared by every AgentMux on the host. A file from the first version
/// holds only a time: that note reported nothing about free space, so it counts as one sent at
/// today's free space and nothing follows until it drops further.
pub fn read_sent(marker: &Path, free_now: u64) -> Option<Sent> {
    let text = std::fs::read_to_string(marker).ok()?;
    Sent::parse(&text).or_else(|| text.trim().parse::<u64>().ok().map(|at| Sent { level: Level::Warning, free: free_now, at }))
}

/// Records that a note went out. Only called once someone received it: a note nobody got must not
/// use up the drop.
pub fn write_sent(marker: &Path, sent: &Sent) -> bool {
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(marker, sent.render()).is_ok()
}

/// Forgets the last note, because the disk recovered.
pub fn clear_sent(marker: &Path) {
    let _ = std::fs::remove_file(marker);
}

/// Whether a note goes out now. `prev` is the last one, `now_level` the level now (not `Ok`),
/// `free` the free bytes now and `now_s` unix seconds.
pub fn should_send(prev: Option<&Sent>, now_level: Level, free: u64, now_s: u64) -> bool {
    if now_level == Level::Ok {
        return false;
    }
    let Some(prev) = prev else { return true };
    if now_level > prev.level {
        return true;
    }
    if now_s.saturating_sub(prev.at) < MIN_GAP_SECS {
        return false;
    }
    free.saturating_add(MORE_DROP) <= prev.free
}

/// Why `free` of `total` is low, in numbers, and which rule tripped: the percentage, the size, or
/// both. Both are shown with the one that did not trip marked as such, so "43 GB free" never sits
/// next to a 20 GB level as if it were below it.
pub fn why(policy: &Policy, level: Level, free: u64, total: u64) -> String {
    let (pct_limit, gb_limit) = if level == Level::Critical {
        (policy.warn_free_pct / 2.0, policy.warn_free_bytes / 2)
    } else {
        (policy.warn_free_pct, policy.warn_free_bytes)
    };
    let kind = if level == Level::Critical { "critical" } else { "warning" };
    let pct = pct_free(free, total);
    let by_pct = pct < pct_limit;
    let by_size = free < gb_limit;
    let pct_part = format!("under {pct_limit:.0}% free ({pct:.1}% is free)");
    let size_part = format!("under {} free ({} is free)", human(gb_limit), human(free));
    match (by_pct, by_size) {
        (true, true) => format!("Both {kind} rules tripped: {pct_part} and {size_part}."),
        (true, false) => format!("The {kind} rule that tripped is the percentage: {pct_part}. The size rule ({}) is not reached.", human(gb_limit)),
        (false, true) => format!("The {kind} rule that tripped is the size: {size_part}. The percentage rule ({pct_limit:.0}%) is not reached."),
        // The level can only be Warning or Critical here because one of the rules tripped.
        (false, false) => format!("The {kind} level was reached ({pct:.1}% / {} free).", human(free)),
    }
}

/// The note: how low the disk is, which rule tripped, and `lines` (the folders it is about) under
/// `heading`.
pub fn note_text(policy: &Policy, level: Level, free: u64, total: u64, heading: &str, lines: &[String]) -> String {
    let low = if level == Level::Critical { "critically low" } else { "running low" };
    let mut out = vec![format!(
        "[AgentMux] This computer's disk is {low}: {} free of {} ({:.1}%). {}",
        human(free),
        human(total),
        pct_free(free, total),
        why(policy, level, free, total)
    )];
    if !lines.is_empty() {
        out.push(heading.to_string());
        out.extend(lines.iter().cloned());
    }
    out.push(
        "AgentMux already clears an agent's incremental cache when it completes a task and prunes old incremental folders every hour. Nothing is needed from you unless you are done with one of these clones: then delete its `target` folder (only your own, by path)."
            .to_string(),
    );
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn pol() -> Policy {
        Policy::default() // warn under 20 GB or 10%
    }

    #[test]
    fn the_level_follows_either_rule_and_critical_is_half_of_it() {
        let total = 440 * GB;
        assert_eq!(level(&pol(), 60 * GB, total), Level::Ok);
        assert_eq!(level(&pol(), 43 * GB, total), Level::Warning, "9.8% is under 10% though 43 GB is over 20 GB");
        assert_eq!(level(&pol(), 44 * GB, total), Level::Ok, "exactly 10% is not under it");
        assert_eq!(level(&pol(), 19 * GB, 100 * GB), Level::Warning, "19% free but under 20 GB");
    }

    #[test]
    fn critical_is_under_half_of_either_level() {
        assert_eq!(level(&pol(), 9 * GB, 1000 * GB), Level::Critical, "under 10 GB");
        assert_eq!(level(&pol(), 21 * GB, 1000 * GB), Level::Critical, "2.1% free is under 5%");
        assert_eq!(level(&pol(), 21 * GB, 200 * GB), Level::Ok, "10.5% free and over 20 GB is fine");
        assert_eq!(level(&pol(), 15 * GB, 200 * GB), Level::Warning, "7.5% free and 15 GB: warning, not critical");
        assert_eq!(level(&pol(), 15 * GB, 0), Level::Ok, "no disk size, no verdict");
    }

    #[test]
    fn a_disk_hovering_at_the_threshold_gets_one_note() {
        // The observed case: about 440 GB, free space 43.7 to 42.4 GB, 9.6 to 9.9%.
        let total = 440 * GB;
        let p = pol();
        let mb = GB / 1000;
        let first = Sent { level: Level::Warning, free: 43_900 * mb, at: 1000 };
        assert!(should_send(None, level(&p, 43_900 * mb, total), 43_900 * mb, 1000), "the first drop is told");
        for (i, f) in [43_700u64, 43_500, 43_300, 42_400, 43_900, 43_100].into_iter().enumerate() {
            let free = f * mb;
            let now = 1000 + 3600 * (i as u64 + 2);
            assert!(!should_send(Some(&first), level(&p, free, total), free, now), "{f} MB free hours later is not a new event");
        }
    }

    #[test]
    fn a_real_further_drop_or_a_worse_level_is_told_again() {
        let p = pol();
        let total = 440 * GB;
        let prev = Sent { level: Level::Warning, free: 44 * GB, at: 1000 };
        assert!(should_send(Some(&prev), level(&p, 38 * GB, total), 38 * GB, 1000 + 4000), "6 GB further down, an hour on");
        assert!(!should_send(Some(&prev), level(&p, 38 * GB, total), 38 * GB, 1000 + 600), "but not within the hour");
        assert!(should_send(Some(&prev), level(&p, 9 * GB, total), 9 * GB, 1000 + 60), "critical is told at once, whatever the hour");
        let crit = Sent { level: Level::Critical, free: 9 * GB, at: 1000 };
        assert!(!should_send(Some(&crit), Level::Warning, 30 * GB, 1000 + 9000), "a better level is not a drop");
        // Free space bouncing across the critical line: the record stays critical, so a dip back
        // into it, hours later, is the same event.
        assert!(!should_send(Some(&crit), Level::Critical, 9 * GB + GB / 2, 1000 + 5 * 3600), "back into critical at about the same free space");
        assert!(should_send(Some(&crit), Level::Critical, 3 * GB, 1000 + 2 * 3600), "but a real further drop is told");
        assert!(!should_send(Some(&prev), Level::Ok, 90 * GB, 99999), "nothing to say when the disk is fine");
    }

    #[test]
    fn recovery_needs_room_to_spare() {
        let p = pol();
        let total = 440 * GB;
        assert!(!recovered(&p, 44 * GB, total), "9.99% is not recovered: just under 11%");
        assert!(!recovered(&p, 46 * GB, total), "10.5% is over the line but not by a point");
        assert!(recovered(&p, 50 * GB, total), "11.4% and over 25 GB");
        assert!(!recovered(&p, 24 * GB, 100 * GB), "24% free but under 25 GB: not by the 5 GB margin");
        assert!(recovered(&p, 26 * GB, 100 * GB), "26% and over 25 GB");
    }

    #[test]
    fn the_record_survives_a_round_trip_and_a_file_from_the_first_version() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("locks").join("build-cache-warn");
        assert!(read_sent(&marker, 5).is_none());
        let sent = Sent { level: Level::Critical, free: 9 * GB, at: 1234 };
        assert!(write_sent(&marker, &sent));
        assert_eq!(read_sent(&marker, 5), Some(sent));
        std::fs::write(&marker, "1793900000").unwrap();
        assert_eq!(
            read_sent(&marker, 42 * GB),
            Some(Sent { level: Level::Warning, free: 42 * GB, at: 1793900000 }),
            "the first version kept only a time: treated as a note sent at today's free space"
        );
        clear_sent(&marker);
        assert!(read_sent(&marker, 5).is_none());
    }

    #[test]
    fn the_text_says_which_rule_tripped_and_which_did_not() {
        let p = pol();
        let by_pct = why(&p, Level::Warning, 43 * GB, 440 * GB);
        assert!(by_pct.contains("percentage"), "{by_pct}");
        assert!(by_pct.contains("under 10% free (9.8% is free)"), "{by_pct}");
        assert!(by_pct.contains("size rule (20.0 GB) is not reached"), "{by_pct}");
        let by_size = why(&p, Level::Warning, 15 * GB, 100 * GB);
        assert!(by_size.contains("rule that tripped is the size"), "{by_size}");
        assert!(by_size.contains("percentage rule (10%) is not reached") || by_size.contains("Both"), "{by_size}");
        let both = why(&p, Level::Warning, 15 * GB, 400 * GB);
        assert!(both.starts_with("Both warning rules tripped"), "{both}");
        let crit = why(&p, Level::Critical, 9 * GB, 100 * GB);
        assert!(crit.contains("critical"), "{crit}");
        assert!(crit.contains("under 10.0 GB free"), "{crit}");
    }

    #[test]
    fn the_note_leads_with_the_numbers_then_the_folders() {
        let p = pol();
        let lines = vec!["- /a/w1/agentmux : 24 GB (3.0 GB of it incremental)".to_string()];
        let text = note_text(&p, Level::Warning, 43 * GB, 440 * GB, "Your largest build folders:", &lines);
        assert!(text.starts_with("[AgentMux] This computer's disk is running low: 43.0 GB free of 440 GB (9.8%)."), "{text}");
        assert!(text.contains("Your largest build folders:\n- /a/w1/agentmux : 24 GB"), "{text}");
        assert!(text.contains("Nothing is needed from you unless"), "an informational note asks nothing: {text}");
        let crit = note_text(&p, Level::Critical, 9 * GB, 440 * GB, "x", &[]);
        assert!(crit.contains("critically low"), "{crit}");
        assert!(!crit.contains("\nx\n"), "no heading without folders: {crit}");
    }
}
