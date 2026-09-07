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
}
