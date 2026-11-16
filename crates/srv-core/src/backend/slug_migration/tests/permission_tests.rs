// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Nothing the migration writes is wider than it should be: a `.mcp.json`
//! stays owner-only through apply, rollback and restore, a new file is
//! owner-only, a replaced file keeps its mode, and the backup and its journal
//! are owner-only. And the journal never holds a key or a secret: it names
//! key rows, whose contents stay in the backup's database copies.

use super::super::run;
use super::fixture::*;

#[test]
fn the_journal_holds_no_key_or_secret() {
    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    run::rollback(&fx.home, Some(&report.journal)).unwrap();
    let journal = std::fs::read_to_string(&report.journal).unwrap();
    for secret in ["hmac-loap-loap-2", "lpriv", "wpriv", "\"secret\"", "keep-me", "private_key"] {
        assert!(!journal.contains(secret), "the journal carries {secret}");
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::super::super::home::write_file;
    use super::super::super::run;
    use super::super::fixture::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn set_mode(path: &Path, m: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(m)).unwrap();
    }

    #[test]
    fn write_file_creates_owner_only_and_keeps_a_replaced_files_mode() {
        let dir = tempfile::tempdir().unwrap();
        let new = dir.path().join("new.json");
        write_file(&new, b"{}").unwrap();
        assert_eq!(mode(&new), 0o600, "a new file is owner-only");

        let shared = dir.path().join("shared.json");
        std::fs::write(&shared, b"old").unwrap();
        set_mode(&shared, 0o640);
        write_file(&shared, b"new").unwrap();
        assert_eq!(mode(&shared), 0o640, "a replaced file keeps its mode");
        assert_eq!(std::fs::read(&shared).unwrap(), b"new");

        let mcp = dir.path().join(".mcp.json");
        std::fs::write(&mcp, b"{}").unwrap();
        set_mode(&mcp, 0o644);
        write_file(&mcp, b"{}").unwrap();
        assert_eq!(mode(&mcp), 0o600, "a .mcp.json holds keys: always owner-only");
    }

    #[test]
    fn a_workspace_mcp_json_stays_owner_only_through_apply_rollback_and_restore() {
        for restore in [false, true] {
            let fx = Fx::standard();
            let mcp = fx.root().join("ws/loap-2/.mcp.json");
            set_mode(&mcp, 0o600);
            let report = super::super::apply(&fx.home).unwrap();
            assert!(std::fs::read_to_string(&mcp).unwrap().contains("\"loap-2\""), "S7 retargeted it");
            assert_eq!(mode(&mcp), 0o600, "after apply");
            if restore {
                run::restore(&fx.home, Some(&report.journal)).unwrap();
            } else {
                run::rollback(&fx.home, Some(&report.journal)).unwrap();
            }
            assert!(std::fs::read_to_string(&mcp).unwrap().contains("loap-loap-2"), "put back");
            assert_eq!(mode(&mcp), 0o600, "after {}", if restore { "restore" } else { "rollback" });
        }
    }

    #[test]
    fn the_backup_and_its_journal_are_owner_only() {
        let fx = Fx::standard();
        let report = super::super::apply(&fx.home).unwrap();
        let dir = Path::new(&report.backup_dir);
        let mut seen = 0;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            assert_eq!(mode(&d), 0o700, "{}", d.display());
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    assert_eq!(mode(&p), 0o600, "{}", p.display());
                    seen += 1;
                }
            }
        }
        assert!(seen > 8, "the databases, the files and the journal: {seen}");
        assert_eq!(mode(Path::new(&report.journal)), 0o600);
    }
}
