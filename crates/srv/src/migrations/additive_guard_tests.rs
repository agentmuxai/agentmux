// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Guard for the shared-store rule (decision D1 of
//! `docs/reports/REPORT_STARTUP_ARCHITECTURE_AND_MIGRATIONS_AUDIT_2026_10_20.md`,
//! `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md` §10.3).
//!
//! Every AgentMux on the computer opens the shared stores at once, whatever its
//! version or channel, so a `MigrationScope::Global` migration must be
//! additive: new tables, columns and rows only, no drops, renames or deletes
//! until an `UPGRADE_FLOOR` raise retires the old shape.
//!
//! This is a light check, not a proof. It reads each Global migration's own
//! source (the part before its `#[cfg(test)]` module, comments left out) and
//! makes two demands: the file declares its effect with a `//! Shared store:`
//! line, and an additive one contains no destructive SQL. Destructive work done
//! through a store helper instead of SQL still needs a reviewer's eye.

use std::path::PathBuf;

use super::{MigrationScope, REGISTRY};

/// Global migrations that shipped before the rule and are not additive. This
/// list only shrinks: a new Global migration must be additive, so it is never
/// added here.
const GRANDFATHERED_NON_ADDITIVE: &[&str] = &["0033_narrow_skill_global_uniqueness_index"];

const DESTRUCTIVE_SQL: &[&str] = &[
    "drop table",
    "drop index",
    "drop column",
    "drop view",
    "drop trigger",
    "rename to",
    "rename column",
    "delete from",
    "truncate",
];

const DECLARATION: &str = "//! Shared store:";

fn source_of(id: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src", "migrations", &format!("m{id}.rs")]
        .iter()
        .collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The migration's own code: before its test module, without comment lines.
fn code_of(source: &str) -> String {
    let body = source.split("#[cfg(test)]").next().unwrap_or(source);
    body.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

fn declared_non_additive(source: &str) -> Option<bool> {
    let line = source.lines().find(|l| l.starts_with(DECLARATION))?;
    Some(line[DECLARATION.len()..].trim_start().to_lowercase().starts_with("not additive"))
}

#[test]
fn every_global_migration_declares_its_shared_store_effect() {
    let missing: Vec<&str> = REGISTRY
        .iter()
        .filter(|m| m.scope() == MigrationScope::Global)
        .filter(|m| declared_non_additive(&source_of(m.id())).is_none())
        .map(|m| m.id())
        .collect();
    assert!(
        missing.is_empty(),
        "Global migrations must start their doc comment with a `{DECLARATION} additive, ...` \
         line (decision D1, see migrations/mod.rs): {missing:?}"
    );
}

#[test]
fn an_additive_global_migration_has_no_destructive_sql() {
    let mut offenders = Vec::new();
    for m in REGISTRY.iter().filter(|m| m.scope() == MigrationScope::Global) {
        let source = source_of(m.id());
        if declared_non_additive(&source) == Some(true) {
            continue;
        }
        let code = code_of(&source);
        for token in DESTRUCTIVE_SQL.iter().filter(|t| code.contains(**t)) {
            offenders.push(format!("{}: `{token}`", m.id()));
        }
    }
    assert!(
        offenders.is_empty(),
        "a Global migration declared additive writes destructive SQL: {offenders:?}. Expand now \
         and remove after an UPGRADE_FLOOR raise (decision D1)"
    );
}

#[test]
fn only_grandfathered_global_migrations_may_declare_non_additive() {
    for m in REGISTRY.iter().filter(|m| m.scope() == MigrationScope::Global) {
        if declared_non_additive(&source_of(m.id())) == Some(true) {
            assert!(
                GRANDFATHERED_NON_ADDITIVE.contains(&m.id()),
                "{} declares itself not additive; a new Global migration must be additive \
                 (decision D1). Only migrations that shipped before the rule are listed in \
                 GRANDFATHERED_NON_ADDITIVE.",
                m.id()
            );
        }
    }
    for id in GRANDFATHERED_NON_ADDITIVE {
        assert!(
            REGISTRY.iter().any(|m| m.id() == *id),
            "{id} left the registry: remove it from GRANDFATHERED_NON_ADDITIVE"
        );
    }
}

#[test]
fn the_destructive_sql_check_flags_what_it_should() {
    let bad = code_of("fn up() { conn.execute(\"DROP INDEX IF EXISTS x\", []); }\n#[cfg(test)]\nmod t {}");
    assert!(DESTRUCTIVE_SQL.iter().any(|t| bad.contains(t)));
    let fine = code_of("// DROP TABLE in a comment\nfn up() { conn.execute(\"INSERT OR IGNORE INTO t VALUES (1)\", []); }\n#[cfg(test)]\nmod t { const S: &str = \"delete from t\"; }");
    assert!(!DESTRUCTIVE_SQL.iter().any(|t| fine.contains(t)));
    assert_eq!(declared_non_additive("//! Shared store: additive, x"), Some(false));
    assert_eq!(declared_non_additive("//! Shared store: not additive (y)"), Some(true));
    assert_eq!(declared_non_additive("//! nothing"), None);
}
