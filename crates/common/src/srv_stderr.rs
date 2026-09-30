//! The stderr line protocol `agentmux-srv` uses to talk to whichever process
//! supervises it — the launcher in packaged builds and `task dev`
//! (`agentmux-launcher/src/srv_spawner.rs`), or the CEF host in
//! `task dev:standalone` (`agentmux-cef/src/sidecar.rs`).
//!
//! srv formats these lines; both supervisors parse them. Putting the
//! vocabulary here means one definition instead of three string literals that
//! have to stay byte-identical by convention (the "keep in sync" pattern
//! `docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2 counts 334
//! instances of).
//!
//! Every line is single-line and prefix-tagged. In startup order:
//!
//! - `AGENTMUXSRV-MIGRATING migrations:<n>` — in-process migrations are about
//!   to run; supervisors extend their ESTART deadline. (Still emitted and
//!   parsed with inline literals at the three sites above — not migrated to
//!   this module yet; only the new line below lives here.)
//! - `AGENTMUXSRV-MIGRATION-BEGIN id:<id> description:<text>` /
//!   `AGENTMUXSRV-MIGRATION-END id:<id> duration_ms:<n>` — one pending
//!   migration is starting / has finished, inside the `AGENTMUXSRV-MIGRATING`
//!   window above. Before these existed, a slow migration was invisible
//!   past the initial count — the splash showed a bare, silently-running
//!   "Backend startup" clock with no indication anything unusual (a
//!   multi-minute migration) was happening inside it. See
//!   `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`
//!   §2.1. Only the launcher supervisor turns these into splash sub-rows
//!   (against the already-open `backend` stage — a migration is not a
//!   separate top-level stage, it happens entirely inside that one); the
//!   CEF-host sidecar has no splash to report to and simply logs them like
//!   any other stderr line, same as it already did for the unparsed line
//!   before this existed.
//! - `AGENTMUXSRV-MIGRATION-FAILED error:<text>` — a migration failed and srv
//!   is about to exit 1 **without** emitting ESTART. Added by Phase 1 of
//!   `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`; before it,
//!   a failed migration was a `warn!` and srv booted anyway against a store in
//!   an unknown state.
//! - `AGENTMUXSRV-ESTART ...` — ready. Parsed separately by each supervisor;
//!   unchanged.

/// Prefix of the line srv writes to stderr immediately before exiting 1
/// because `run_pending_migrations` failed. Supervisors match on this to fail
/// fast with the real reason instead of waiting out the ESTART timeout.
pub const MIGRATION_FAILED_PREFIX: &str = "AGENTMUXSRV-MIGRATION-FAILED";

/// Format the failure line srv emits. The error text is flattened to a single
/// line (the protocol is line-delimited; a multi-line SQLite error would
/// otherwise split into one tagged line and several untagged ones).
pub fn migration_failed_line(err: &str) -> String {
    format!("{} error:{}", MIGRATION_FAILED_PREFIX, one_line(err))
}

/// Parse a stderr line. Returns the error message if this is a
/// `MIGRATION_FAILED_PREFIX` line, `None` for any other line. Tolerates a
/// missing `error:` field and never returns an empty message, so callers can
/// put the result straight into a user-facing dialog.
pub fn parse_migration_failed_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix(MIGRATION_FAILED_PREFIX)?;
    let rest = rest.trim_start();
    let msg = rest.strip_prefix("error:").unwrap_or(rest).trim();
    Some(if msg.is_empty() { "unknown error".to_string() } else { msg.to_string() })
}

fn one_line(s: &str) -> String {
    s.split(['\n', '\r'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Prefix of the line srv writes to stderr just before applying one pending
/// migration.
pub const MIGRATION_BEGIN_PREFIX: &str = "AGENTMUXSRV-MIGRATION-BEGIN";
/// Prefix of the line srv writes to stderr just after one migration applies
/// successfully. (No `-END` line is written on failure — `up()` returning
/// `Err` goes straight to `AGENTMUXSRV-MIGRATION-FAILED` and srv exits.)
pub const MIGRATION_END_PREFIX: &str = "AGENTMUXSRV-MIGRATION-END";

/// A parsed `MIGRATION_BEGIN_PREFIX` line.
pub struct MigrationBegin {
    pub id: String,
    pub description: String,
}

/// A parsed `MIGRATION_END_PREFIX` line.
pub struct MigrationEnd {
    pub id: String,
    pub duration_ms: u64,
}

/// Migration ids (`m.id()`, e.g. `"0020_agent_color_backfill"`) are a fixed,
/// engineer-chosen identifier format with no spaces — never free text — so
/// splitting `id:<id> description:<text>` on the first space is safe and
/// exact, unlike `description` itself which is flattened via `one_line`.
pub fn migration_begin_line(id: &str, description: &str) -> String {
    format!("{} id:{} description:{}", MIGRATION_BEGIN_PREFIX, id, one_line(description))
}

pub fn migration_end_line(id: &str, duration_ms: u64) -> String {
    format!("{} id:{} duration_ms:{}", MIGRATION_END_PREFIX, id, duration_ms)
}

/// Parse a `MIGRATION_BEGIN_PREFIX` line. `None` for any other line, or a
/// begin line missing the `id:` field it cannot be meaningfully split
/// without.
pub fn parse_migration_begin_line(line: &str) -> Option<MigrationBegin> {
    let rest = line.strip_prefix(MIGRATION_BEGIN_PREFIX)?.trim_start();
    let rest = rest.strip_prefix("id:")?;
    let (id, rest) = rest.split_once(' ')?;
    let description = rest.strip_prefix("description:").unwrap_or(rest).trim();
    Some(MigrationBegin { id: id.to_string(), description: description.to_string() })
}

/// Parse a `MIGRATION_END_PREFIX` line. `None` for any other line, a
/// malformed one, or a non-numeric/missing `duration_ms:` field.
pub fn parse_migration_end_line(line: &str) -> Option<MigrationEnd> {
    let rest = line.strip_prefix(MIGRATION_END_PREFIX)?.trim_start();
    let rest = rest.strip_prefix("id:")?;
    let (id, rest) = rest.split_once(' ')?;
    let ms_str = rest.trim().strip_prefix("duration_ms:")?;
    let duration_ms = ms_str.trim().parse::<u64>().ok()?;
    Some(MigrationEnd { id: id.to_string(), duration_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_is_prefixed_and_single_line() {
        let line = migration_failed_line("migration 0007 failed:\r\n  FOREIGN KEY constraint failed\n");
        assert_eq!(
            line,
            "AGENTMUXSRV-MIGRATION-FAILED error:migration 0007 failed: | FOREIGN KEY constraint failed"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn round_trips_through_parse() {
        let msg = "run_pending_migrations: open shared store: disk I/O error";
        assert_eq!(parse_migration_failed_line(&migration_failed_line(msg)).as_deref(), Some(msg));
    }

    #[test]
    fn parse_tolerates_missing_error_field() {
        assert_eq!(
            parse_migration_failed_line("AGENTMUXSRV-MIGRATION-FAILED something broke").as_deref(),
            Some("something broke")
        );
    }

    #[test]
    fn parse_never_returns_empty_message() {
        assert_eq!(parse_migration_failed_line("AGENTMUXSRV-MIGRATION-FAILED").as_deref(), Some("unknown error"));
        assert_eq!(parse_migration_failed_line("AGENTMUXSRV-MIGRATION-FAILED error:").as_deref(), Some("unknown error"));
    }

    #[test]
    fn parse_ignores_other_protocol_lines() {
        assert_eq!(parse_migration_failed_line("AGENTMUXSRV-MIGRATING migrations:3"), None);
        assert_eq!(parse_migration_failed_line("AGENTMUXSRV-ESTART ws:127.0.0.1:1 web:127.0.0.1:2"), None);
        assert_eq!(parse_migration_failed_line("plain stderr noise"), None);
    }

    #[test]
    fn migrating_prefix_is_not_a_prefix_of_failed() {
        // Both start with "AGENTMUXSRV-MIGRATI"; the supervisors check
        // `starts_with` for each, so neither may be a prefix of the other.
        assert!(!MIGRATION_FAILED_PREFIX.starts_with("AGENTMUXSRV-MIGRATING"));
        assert!(!"AGENTMUXSRV-MIGRATING".starts_with(MIGRATION_FAILED_PREFIX));
    }

    #[test]
    fn none_of_the_four_migration_prefixes_are_a_prefix_of_another() {
        // All four share "AGENTMUXSRV-MIGRAT"; every supervisor matches with
        // `starts_with`/`strip_prefix`, so a false-positive prefix match
        // would misroute a real line.
        let all = [
            "AGENTMUXSRV-MIGRATING",
            MIGRATION_FAILED_PREFIX,
            MIGRATION_BEGIN_PREFIX,
            MIGRATION_END_PREFIX,
        ];
        for a in all {
            for b in all {
                if a == b { continue; }
                assert!(!a.starts_with(b), "{a:?} must not start with {b:?}");
            }
        }
    }

    #[test]
    fn begin_line_round_trips_through_parse() {
        let line = migration_begin_line("0020_agent_color_backfill", "backfill default agent colors");
        let parsed = parse_migration_begin_line(&line).expect("must parse");
        assert_eq!(parsed.id, "0020_agent_color_backfill");
        assert_eq!(parsed.description, "backfill default agent colors");
    }

    #[test]
    fn begin_line_flattens_a_multiline_description() {
        let line = migration_begin_line("0007_x", "line one\nline two");
        assert!(!line.contains('\n'));
        assert_eq!(parse_migration_begin_line(&line).unwrap().description, "line one | line two");
    }

    #[test]
    fn end_line_round_trips_through_parse() {
        let line = migration_end_line("0020_agent_color_backfill", 147);
        let parsed = parse_migration_end_line(&line).expect("must parse");
        assert_eq!(parsed.id, "0020_agent_color_backfill");
        assert_eq!(parsed.duration_ms, 147);
    }

    #[test]
    fn begin_and_end_parsers_ignore_other_protocol_lines_and_each_other() {
        assert!(parse_migration_begin_line("AGENTMUXSRV-MIGRATING migrations:3").is_none());
        assert!(parse_migration_begin_line("AGENTMUXSRV-ESTART ws:x web:y").is_none());
        assert!(parse_migration_begin_line(&migration_end_line("0007_x", 5)).is_none());
        assert!(parse_migration_end_line(&migration_begin_line("0007_x", "d")).is_none());
    }

    #[test]
    fn malformed_begin_and_end_lines_do_not_panic() {
        assert!(parse_migration_begin_line(MIGRATION_BEGIN_PREFIX).is_none());
        assert!(parse_migration_begin_line(&format!("{MIGRATION_BEGIN_PREFIX} garbage")).is_none());
        assert!(parse_migration_end_line(MIGRATION_END_PREFIX).is_none());
        assert!(parse_migration_end_line(&format!("{MIGRATION_END_PREFIX} id:0007_x duration_ms:notanumber")).is_none());
    }
}
