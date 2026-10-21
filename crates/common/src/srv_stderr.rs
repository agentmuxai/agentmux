//! The stderr line protocol `agentmux-srv` uses to talk to whichever process
//! supervises it — the launcher in packaged builds and `task dev`
//! (`crates/launcher/src/srv_spawner.rs`), or the CEF host in
//! `task dev:standalone` (`crates/cef/src/sidecar.rs`).
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
//!   to run; supervisors extend their ESTART deadline. A new version's first
//!   start prints `AGENTMUXSRV-MIGRATING copying-previous-version:<v>` before
//!   copying the previous version's databases (`bootstrap/carry_forward.rs`);
//!   supervisors match the prefix only ([`migrating_line`],
//!   [`is_migrating_line`]).
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
//! - `AGENTMUXSRV-STEP-BEGIN id:<id> label:<text>` /
//!   `AGENTMUXSRV-STEP-END id:<id> duration_ms:<n>[ status:error]` — one data-preparation
//!   step besides a migration: copying the previous version's data, a
//!   snapshot, waiting for another AgentMux's migrations, the backup. Written
//!   by srv's startup and by `agentmux-srv migrate --prepare`; the launcher
//!   shows each as a sub-row of `backend`, like a migration.
//! - `AGENTMUXSRV-NEEDS-PREPARE reason:<text>` — srv was started by a
//!   supervisor that prepares data ([`SUPERVISOR_PREPARES_ENV`]) and found data
//!   that needs preparing (a previous version to copy forward, or pending
//!   migrations); it exits [`NEEDS_PREPARE_EXIT`] before writing anything, and
//!   the supervisor runs `agentmux-srv migrate --prepare`, then starts it again.
//! - `AGENTMUXSRV-MIGRATION-FAILED error:<text>` — a migration failed and srv
//!   is about to exit 1 **without** emitting ESTART. Added by Phase 1 of
//!   `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`; before it,
//!   a failed migration was a `warn!` and srv booted anyway against a store in
//!   an unknown state.
//! - `AGENTMUXSRV-ESTART ws:<addr> web:<addr> version:<v> buildtime:<t>
//!   instance:<id> pending_migrations:<n>` — ready ([`Estart`]).
//! - `AGENTMUXSRV-EVENT:<json>` — a startup event for the launcher to log
//!   ([`parse_event_line`]).
//! - `AGENTMUXSRV-UPGRADE version:<v> launcher:<path>` (a portable) or
//!   `… installer:<path>` (a `setup.exe` install) — any time after ESTART: the
//!   user chose "Upgrade & restart" and `<path>` is the launcher, or the
//!   installer, of a release srv downloaded and verified. The launcher checks it again, asks
//!   the user itself, and restarts into it (`crates/launcher/src/upgrade_restart.rs`).

/// Prefix of the line srv writes when it is ready to serve.
pub const ESTART_PREFIX: &str = "AGENTMUXSRV-ESTART";

/// The fields of an `ESTART_PREFIX` line. Endpoints are `host:port`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Estart {
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub version: String,
    pub build_time: String,
    pub instance_id: String,
    /// Migrations still pending after srv's own run; non-zero only when one
    /// could not be applied.
    pub pending_migrations: usize,
}

pub fn estart_line(e: &Estart) -> String {
    format!(
        "{} ws:{} web:{} version:{} buildtime:{} instance:{} pending_migrations:{}",
        ESTART_PREFIX, e.ws_endpoint, e.web_endpoint, e.version, e.build_time, e.instance_id, e.pending_migrations
    )
}

/// Parse an `ESTART_PREFIX` line; `None` for any other line. A missing field
/// reads as empty (or 0), as both supervisors always read it.
pub fn parse_estart_line(line: &str) -> Option<Estart> {
    let rest = line.strip_prefix(ESTART_PREFIX)?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let get = |prefix: &str| parts.iter().find_map(|p| p.strip_prefix(prefix)).unwrap_or_default().to_string();
    Some(Estart {
        ws_endpoint: get("ws:"),
        web_endpoint: get("web:"),
        version: get("version:"),
        build_time: get("buildtime:"),
        instance_id: get("instance:"),
        pending_migrations: get("pending_migrations:").parse().unwrap_or(0),
    })
}

/// Prefix of the line srv writes before slow work that precedes ESTART
/// (migrations, copying the previous version's data); supervisors extend
/// their ESTART deadline when they see it.
pub const MIGRATING_PREFIX: &str = "AGENTMUXSRV-MIGRATING";

/// `AGENTMUXSRV-MIGRATING <what>:<detail>`, e.g. `migrations:3`.
pub fn migrating_line(what: &str, detail: impl std::fmt::Display) -> String {
    format!("{MIGRATING_PREFIX} {what}:{detail}")
}

pub fn is_migrating_line(line: &str) -> bool {
    line.starts_with(MIGRATING_PREFIX)
}

/// Prefix of a startup event line; the rest of the line is its JSON.
pub const EVENT_PREFIX: &str = "AGENTMUXSRV-EVENT:";

/// The JSON of an `EVENT_PREFIX` line; `None` for any other line.
pub fn parse_event_line(line: &str) -> Option<&str> {
    line.strip_prefix(EVENT_PREFIX)
}

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

/// Set to `1` by a supervisor that runs `agentmux-srv migrate --prepare`
/// itself (the launcher). srv then never copies, snapshots or migrates at boot:
/// it checks read-only and exits [`NEEDS_PREPARE_EXIT`] when the data needs it.
/// Without it (headless srv, `task dev:standalone`) srv prepares in-process.
pub const SUPERVISOR_PREPARES_ENV: &str = "AGENTMUX_SUPERVISOR_PREPARES";

/// srv's exit code after an `AGENTMUXSRV-NEEDS-PREPARE` line.
pub const NEEDS_PREPARE_EXIT: i32 = 4;

/// Prefix of the line srv writes, under [`SUPERVISOR_PREPARES_ENV`], before
/// exiting because its data needs `migrate --prepare`.
pub const NEEDS_PREPARE_PREFIX: &str = "AGENTMUXSRV-NEEDS-PREPARE";

pub fn needs_prepare_line(reason: &str) -> String {
    format!("{NEEDS_PREPARE_PREFIX} reason:{}", one_line(reason))
}

/// The reason of a `NEEDS_PREPARE_PREFIX` line; `None` for any other line.
pub fn parse_needs_prepare_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix(NEEDS_PREPARE_PREFIX)?.trim_start();
    Some(rest.strip_prefix("reason:").unwrap_or(rest).trim().to_string())
}

/// Prefix of the line srv writes when a data-preparation step starts.
pub const STEP_BEGIN_PREFIX: &str = "AGENTMUXSRV-STEP-BEGIN";
/// Prefix of the line srv writes when that step ends.
pub const STEP_END_PREFIX: &str = "AGENTMUXSRV-STEP-END";

/// A parsed `STEP_BEGIN_PREFIX` line. Ids are code-chosen, without spaces.
#[derive(Debug, PartialEq, Eq)]
pub struct StepBegin {
    pub id: String,
    pub label: String,
}

/// A parsed `STEP_END_PREFIX` line.
#[derive(Debug, PartialEq, Eq)]
pub struct StepEnd {
    pub id: String,
    pub duration_ms: u64,
    /// `false` when the step failed (`status:error` on the line).
    pub ok: bool,
}

pub fn step_begin_line(id: &str, label: &str) -> String {
    format!("{STEP_BEGIN_PREFIX} id:{id} label:{}", one_line(label))
}

pub fn step_end_line(id: &str, duration_ms: u64, ok: bool) -> String {
    let status = if ok { "" } else { " status:error" };
    format!("{STEP_END_PREFIX} id:{id} duration_ms:{duration_ms}{status}")
}

pub fn parse_step_begin_line(line: &str) -> Option<StepBegin> {
    let rest = line.strip_prefix(STEP_BEGIN_PREFIX)?.trim_start().strip_prefix("id:")?;
    let (id, rest) = rest.split_once(' ')?;
    let label = rest.strip_prefix("label:").unwrap_or(rest).trim();
    Some(StepBegin { id: id.to_string(), label: label.to_string() })
}

pub fn parse_step_end_line(line: &str) -> Option<StepEnd> {
    let rest = line.strip_prefix(STEP_END_PREFIX)?.trim_start().strip_prefix("id:")?;
    let (id, rest) = rest.split_once(' ')?;
    let mut fields = rest.split_whitespace();
    let duration_ms = fields.next()?.strip_prefix("duration_ms:")?.parse().ok()?;
    let ok = !fields.any(|f| f == "status:error");
    Some(StepEnd { id: id.to_string(), duration_ms, ok })
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

/// Prefix of the line srv writes when the user chose "Upgrade & restart".
pub const UPGRADE_PREFIX: &str = "AGENTMUXSRV-UPGRADE";

/// What the launcher leaves in `updates/` of the data folder when the user
/// answered No to the restart question, or the question lapsed: the version it
/// asked about. srv reads it to stop saying "Restarting" (the launcher has no
/// other way to tell it), so the user can start the upgrade again at once.
pub const UPGRADE_DECLINED_FILE: &str = "declined.txt";

/// What an upgrade restarts into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeTarget {
    /// A portable's new folder: start this launcher.
    Launcher(std::path::PathBuf),
    /// A `setup.exe` install: run this installer, then start AgentMux again.
    /// `sha256` (lowercase hex) is the release's digest: the launcher checks
    /// it again on the very file it runs.
    Installer { installer: std::path::PathBuf, sha256: String },
}

/// A parsed `UPGRADE_PREFIX` line: upgrade to `version` through `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeRequest {
    pub version: String,
    pub target: UpgradeTarget,
}

/// The path goes last: it can hold spaces.
pub fn upgrade_line(version: &str, target: &UpgradeTarget) -> String {
    match target {
        UpgradeTarget::Launcher(p) => format!("{} version:{} launcher:{}", UPGRADE_PREFIX, version, p.display()),
        UpgradeTarget::Installer { installer, sha256 } => {
            format!("{} version:{} sha256:{} installer:{}", UPGRADE_PREFIX, version, sha256, installer.display())
        }
    }
}

/// Parse an `UPGRADE_PREFIX` line. `None` for any other line, or one missing
/// a field (an installer needs its 64-hex-digit `sha256:`).
pub fn parse_upgrade_line(line: &str) -> Option<UpgradeRequest> {
    let rest = line.strip_prefix(UPGRADE_PREFIX)?.trim_start();
    let (version, rest) = rest.strip_prefix("version:")?.split_once(' ')?;
    let rest = rest.trim_start();
    let target = if let Some(p) = rest.strip_prefix("launcher:") {
        UpgradeTarget::Launcher(p.trim().into())
    } else {
        let (sha256, rest) = rest.strip_prefix("sha256:")?.split_once(' ')?;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let installer = rest.trim_start().strip_prefix("installer:")?.trim().into();
        UpgradeTarget::Installer { installer, sha256: sha256.to_ascii_lowercase() }
    };
    let empty = match &target {
        UpgradeTarget::Launcher(p) | UpgradeTarget::Installer { installer: p, .. } => p.as_os_str().is_empty(),
    };
    if version.is_empty() || empty {
        return None;
    }
    Some(UpgradeRequest { version: version.to_string(), target })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estart_line_round_trips() {
        let e = Estart {
            ws_endpoint: "127.0.0.1:61001".into(),
            web_endpoint: "127.0.0.1:61002".into(),
            version: "0.59.34".into(),
            build_time: "20261020.101010".into(),
            instance_id: "inst-1".into(),
            pending_migrations: 2,
        };
        let line = estart_line(&e);
        assert_eq!(
            line,
            "AGENTMUXSRV-ESTART ws:127.0.0.1:61001 web:127.0.0.1:61002 version:0.59.34 buildtime:20261020.101010 instance:inst-1 pending_migrations:2"
        );
        assert_eq!(parse_estart_line(&line), Some(e));
        assert_eq!(parse_estart_line("AGENTMUXSRV-MIGRATING migrations:1"), None);
        let bare = parse_estart_line("AGENTMUXSRV-ESTART ws:127.0.0.1:1").unwrap();
        assert_eq!((bare.ws_endpoint.as_str(), bare.pending_migrations), ("127.0.0.1:1", 0));
    }

    #[test]
    fn step_lines_round_trip_and_stay_apart_from_migration_lines() {
        let begin = step_begin_line("copy_previous_version", "Copying your data\nfrom 0.59.33");
        assert_eq!(
            parse_step_begin_line(&begin),
            Some(StepBegin { id: "copy_previous_version".into(), label: "Copying your data | from 0.59.33".into() })
        );
        let end = step_end_line("backup", 812, true);
        assert_eq!(parse_step_end_line(&end), Some(StepEnd { id: "backup".into(), duration_ms: 812, ok: true }));
        let failed = step_end_line("copy_previous_version", 40, false);
        assert_eq!(failed, "AGENTMUXSRV-STEP-END id:copy_previous_version duration_ms:40 status:error");
        assert_eq!(parse_step_end_line(&failed).map(|e| e.ok), Some(false));
        assert_eq!(parse_step_begin_line(&migration_begin_line("0001_x", "y")), None);
        assert!(parse_migration_begin_line(&begin).is_none());
        assert!(!is_migrating_line(&begin));
        assert_eq!(parse_step_end_line("AGENTMUXSRV-STEP-END id:backup duration_ms:x"), None);
    }

    #[test]
    fn needs_prepare_line_round_trips() {
        let line = needs_prepare_line("2 migration(s) pending,\nfirst 0038_x");
        assert_eq!(parse_needs_prepare_line(&line).as_deref(), Some("2 migration(s) pending, | first 0038_x"));
        assert_eq!(parse_needs_prepare_line(&migrating_line("migrations", 2)), None);
        assert!(parse_migration_failed_line(&line).is_none());
    }

    #[test]
    fn migrating_and_event_lines() {
        assert_eq!(migrating_line("migrations", 3), "AGENTMUXSRV-MIGRATING migrations:3");
        assert!(is_migrating_line(&migrating_line("copying-previous-version", "0.59.33")));
        assert!(!is_migrating_line("AGENTMUXSRV-MIGRATION-BEGIN id:x description:y"));
        assert_eq!(parse_event_line(r#"AGENTMUXSRV-EVENT:{"a":1}"#), Some(r#"{"a":1}"#));
        assert_eq!(parse_event_line("AGENTMUXSRV-ESTART ws:x"), None);
    }

    #[test]
    fn upgrade_line_round_trips_a_path_with_spaces() {
        let path = std::path::Path::new(r"C:\Users\A User\Apps\agentmux-0.59.27-x64-portable\agentmux.exe");
        let target = UpgradeTarget::Launcher(path.to_path_buf());
        let line = upgrade_line("0.59.27", &target);
        assert_eq!(parse_upgrade_line(&line), Some(UpgradeRequest { version: "0.59.27".into(), target }));
        let sha = "ab".repeat(32);
        let setup = UpgradeTarget::Installer {
            installer: r"C:\Users\A User\updates\0.59.27\AgentMux-0.59.27-x64-setup.exe".into(),
            sha256: sha.clone(),
        };
        assert_eq!(
            parse_upgrade_line(&upgrade_line("0.59.27", &setup)),
            Some(UpgradeRequest { version: "0.59.27".into(), target: setup })
        );
        assert_eq!(parse_upgrade_line(&format!("AGENTMUXSRV-UPGRADE version:0.59.27 sha256:{sha} installer:")), None);
        // An installer without its digest, or with a malformed one, is refused.
        assert_eq!(parse_upgrade_line("AGENTMUXSRV-UPGRADE version:0.59.27 installer:C:/u/setup.exe"), None);
        assert_eq!(parse_upgrade_line("AGENTMUXSRV-UPGRADE version:0.59.27 sha256:ab installer:C:/u/setup.exe"), None);
        assert_eq!(parse_upgrade_line("AGENTMUXSRV-UPGRADE version:0.59.27"), None);
        assert_eq!(parse_upgrade_line("AGENTMUXSRV-UPGRADE launcher:C:/x/agentmux.exe"), None);
        assert_eq!(parse_upgrade_line("AGENTMUXSRV-MIGRATING migrations:1"), None);
    }

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
