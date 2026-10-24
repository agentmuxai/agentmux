// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The file steps: S1 (the shared definition, through its record lock), the
//! workspace `.mcp.json` half of S7, and S8 (the `gh-<slug>` folders, the
//! bashwrap cwd file, stale registrations). Writes go through a temporary file
//! and a rename. An undo restores the backup's bytes when the file is still
//! what the step left, so a reverted file is byte-identical; a file changed
//! since is reverted field by field (S1) or left as it is, and said so.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::home::{fold, write_file, Home};
use super::steps::{parse, to_value, Agent, Ctx};
use crate::registry::{record_bytes, DefStoreError, DefinitionStore};

fn io(msg: String) -> DefStoreError {
    DefStoreError::Io(std::io::Error::other(msg))
}

pub(super) fn def_path(home: &Home, uid: &str) -> PathBuf {
    home.definitions().join(format!("{uid}.json"))
}

// ── S1: the shared definition ────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct S1Before {
    slug: String,
    had_former: bool,
}

fn former_has(value: &Value, slug: &str) -> bool {
    value
        .pointer("/data/former_slugs")
        .and_then(Value::as_array)
        .is_some_and(|l| l.iter().any(|f| f.get("slug").and_then(Value::as_str).is_some_and(|s| fold(s) == fold(slug))))
}

/// `value` renamed: slug the new one, the old one appended to the former
/// slugs (lowercased, `retired_at` the run's). Whether it changed.
fn renamed(value: &mut Value, a: &Agent, retired_at: i64) -> Result<bool, String> {
    let had = former_has(value, &a.old);
    let data = value.get_mut("data").and_then(Value::as_object_mut).ok_or("the definition has no `data`")?;
    let mut changed = false;
    if data.get("slug").and_then(Value::as_str) != Some(a.new.as_str()) {
        data.insert("slug".into(), Value::String(a.new.clone()));
        changed = true;
    }
    if !had {
        let list = data.entry("former_slugs").or_insert_with(|| Value::Array(Vec::new()));
        let list = list.as_array_mut().ok_or("`former_slugs` isn't a list")?;
        list.push(serde_json::json!({ "slug": fold(&a.old), "retired_at": retired_at }));
        changed = true;
    }
    Ok(changed)
}

fn read_json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("the definition isn't JSON: {e}"))
}

pub(super) fn s1_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let path = def_path(ctx.home, &a.uid);
    let value = read_json(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    let slug = value.pointer("/data/slug").and_then(Value::as_str).unwrap_or_default().to_string();
    to_value(&S1Before { slug, had_former: former_has(&value, &a.old) })
}

fn with_def<T>(ctx: &Ctx, a: &Agent, f: impl FnOnce(&crate::registry::LockedRecord<'_>) -> Result<T, String>) -> Result<T, String> {
    let store = DefinitionStore::open(ctx.home.definitions()).map_err(|e| e.to_string())?;
    store.with_record_locked(&a.uid, |rec| f(rec).map_err(io)).map_err(|e| e.to_string())
}

pub(super) fn s1_apply(ctx: &Ctx, a: &Agent, _before: &Value) -> Result<Vec<String>, String> {
    with_def(ctx, a, |rec| {
        let bytes = rec.raw().map_err(|e| e.to_string())?.ok_or("the agent's shared definition is gone")?;
        let mut value = read_json(&bytes)?;
        if !renamed(&mut value, a, ctx.retired_at)? {
            return Ok(Vec::new());
        }
        rec.replace_raw(&record_bytes(&value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(vec![format!("shared definition: slug {} -> {}, {} kept as a former slug", a.old, a.new, fold(&a.old))])
    })
}

pub(super) fn s1_undo(ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let b: S1Before = parse(before)?;
    let original = std::fs::read(ctx.backup_of(&def_path(ctx.home, &a.uid))?).map_err(|e| e.to_string())?;
    with_def(ctx, a, |rec| {
        let Some(bytes) = rec.raw().map_err(|e| e.to_string())? else {
            return Ok(vec!["shared definition: gone since the run (deleted); nothing to revert".into()]);
        };
        let current = read_json(&bytes)?;
        let mut expected = read_json(&original)?;
        renamed(&mut expected, a, ctx.retired_at)?;
        if current == expected || bytes == original {
            if bytes != original {
                rec.replace_raw(&original).map_err(|e| e.to_string())?;
            }
            return Ok(vec!["shared definition: restored exactly as it was".into()]);
        }
        let mut value = current;
        let data = value.get_mut("data").and_then(Value::as_object_mut).ok_or("the definition has no `data`")?;
        data.insert("slug".into(), Value::String(b.slug.clone()));
        if !b.had_former {
            if let Some(list) = data.get_mut("former_slugs").and_then(Value::as_array_mut) {
                list.retain(|f| f.get("slug").and_then(Value::as_str).is_none_or(|s| fold(s) != fold(&a.old)));
                if list.is_empty() {
                    data.remove("former_slugs");
                }
            }
        }
        rec.replace_raw(&record_bytes(&value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(vec![format!("shared definition: slug back to {}; edited since the run, so its other changes are kept", b.slug)])
    })
}

// ── S7: the workspace .mcp.json ──────────────────────────────────────

/// The `.mcp.json` files that sign as the old id: in the definition's
/// working folder and in each of the agent's panes' `cmd:cwd`.
pub(super) fn mcp_files(ctx: &Ctx, a: &Agent) -> Result<Vec<PathBuf>, String> {
    let def = std::fs::read(def_path(ctx.home, &a.uid)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let mut dirs: Vec<String> =
        def.and_then(|d| d.pointer("/data/working_directory").and_then(Value::as_str).map(str::to_string)).into_iter().collect();
    dirs.extend(super::steps_db::block_cwds(ctx, a)?);
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in dirs.iter().filter(|d| !d.trim().is_empty()) {
        let path = PathBuf::from(crate::backend::blockcontroller::core::expand_home_dir(dir)).join(".mcp.json");
        if !out.contains(&path) && names(&path, &a.old) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Whether the file's AgentMux entry signs as `id`.
fn names(path: &Path, id: &str) -> bool {
    let Some(json) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return false };
    json.pointer("/mcpServers/agentmux/env/AGENTMUX_AGENT_ID").and_then(Value::as_str).is_some_and(|v| fold(v) == fold(id))
}

/// `content` with its AgentMux entry signing as the new id, and the old
/// id's keys dropped: the next spawn writes the new id's
/// (`mcp_json_identity::realign_workspace_mcp_json`). The rest of the file,
/// the user's own servers included, is kept.
fn retargeted(content: &str, a: &Agent) -> Option<String> {
    crate::backend::mcp_json_identity::set_agent_id(content, &a.new).map(|(patched, _)| patched)
}

pub(super) fn s7_files_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let files: Vec<String> = mcp_files(ctx, a)?.iter().map(|p| p.display().to_string()).collect();
    to_value(&files)
}

pub(super) fn s7_files_apply(a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for file in parse::<Vec<String>>(before)? {
        let Ok(content) = std::fs::read_to_string(&file) else { continue };
        if !names(Path::new(&file), &a.old) {
            continue;
        }
        if let Some(patched) = retargeted(&content, a) {
            write_file(Path::new(&file), patched.as_bytes())?;
            changes.push(format!("{file}: signs as {} (its keys come at the next spawn)", a.new));
        }
    }
    Ok(changes)
}

pub(super) fn s7_files_undo(ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for file in parse::<Vec<String>>(before)? {
        let path = Path::new(&file);
        let original = std::fs::read(ctx.backup_of(path)?).map_err(|e| e.to_string())?;
        let current = std::fs::read(path).ok();
        let left_by_run = String::from_utf8(original.clone()).ok().and_then(|o| retargeted(&o, a)).map(String::into_bytes);
        if current.as_deref() == Some(original.as_slice()) {
            continue;
        }
        if current.is_none() || current == left_by_run {
            write_file(path, &original)?;
            changes.push(format!("{file}: restored exactly as it was"));
        } else {
            changes.push(format!("{file}: changed since the run (a relaunch rewrites it), left as it is"));
        }
    }
    Ok(changes)
}

// ── S8: folders and files named after the slug ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Move {
    from: String,
    to: String,
    to_existed: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct S8Before {
    moves: Vec<Move>,
    /// Stale registrations, deleted; their copies are in the backup.
    deletes: Vec<String>,
}

/// bashwrap's file name for an id (`cwd_state.rs`).
fn cwd_key(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// The stale registrations S8 deletes: each version's `data/agents/<old>.json`
/// and the cross-channel registry's `<old>/` entries.
pub(super) fn stale_registrations(ctx: &Ctx, a: &Agent) -> Vec<PathBuf> {
    let stem = agentmux_common::slug::file_stem(&a.old);
    let mut out: Vec<PathBuf> = ctx
        .installs
        .iter()
        .flat_map(|i| i.versions.iter().map(|v| v.registrations().join(format!("{stem}.json"))))
        .filter(|p| p.is_file())
        .collect();
    if let Ok(files) = std::fs::read_dir(ctx.home.reactive().join(&stem)) {
        let mut shared: Vec<PathBuf> = files.flatten().map(|f| f.path()).filter(|p| p.is_file()).collect();
        shared.sort();
        out.extend(shared);
    }
    out
}

pub(super) fn cwd_file(home: &Home, id: &str) -> PathBuf {
    home.bashwrap_dir.join(format!("{}.cwd", cwd_key(id)))
}

pub(super) fn s8_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut moves = Vec::new();
    for install in ctx.installs {
        let from = crate::backend::gh_guard::agent_gh_config_dir(&install.config_dir(), Some(&a.old));
        let to = crate::backend::gh_guard::agent_gh_config_dir(&install.config_dir(), Some(&a.new));
        if from.is_dir() {
            moves.push(Move { to_existed: to.exists(), from: from.display().to_string(), to: to.display().to_string() });
        }
    }
    let (from, to) = (cwd_file(ctx.home, &a.old), cwd_file(ctx.home, &a.new));
    if from.is_file() {
        moves.push(Move { to_existed: to.exists(), from: from.display().to_string(), to: to.display().to_string() });
    }
    let deletes = stale_registrations(ctx, a).iter().map(|p| p.display().to_string()).collect();
    to_value(&S8Before { moves, deletes })
}

pub(super) fn s8_apply(before: &Value) -> Result<Vec<String>, String> {
    let b: S8Before = parse(before)?;
    let mut changes = Vec::new();
    for m in &b.moves {
        let (from, to) = (Path::new(&m.from), Path::new(&m.to));
        if !from.exists() {
            continue;
        }
        if to.exists() {
            tracing::warn!(from = %m.from, to = %m.to, "fork slug migration: the new name's folder exists; both kept");
            changes.push(format!("{} kept beside {}, which already existed", m.from, m.to));
            continue;
        }
        std::fs::rename(from, to).map_err(|e| format!("move {} to {}: {e}", m.from, m.to))?;
        changes.push(format!("moved {} to {}", m.from, m.to));
    }
    for file in &b.deletes {
        let path = Path::new(file);
        match std::fs::remove_file(path) {
            Ok(()) => changes.push(format!("deleted the stale registration {file}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("delete {file}: {e}")),
        }
        if let Some(dir) = path.parent().filter(|d| d.parent().is_some_and(|p| p.ends_with("reactive"))) {
            let _ = std::fs::remove_dir(dir);
        }
    }
    Ok(changes)
}

pub(super) fn s8_undo(ctx: &Ctx, before: &Value) -> Result<Vec<String>, String> {
    let b: S8Before = parse(before)?;
    let mut changes = Vec::new();
    for m in b.moves.iter().rev() {
        let (from, to) = (Path::new(&m.from), Path::new(&m.to));
        if !m.to_existed && to.exists() && !from.exists() {
            std::fs::rename(to, from).map_err(|e| format!("move {} back to {}: {e}", m.to, m.from))?;
            changes.push(format!("moved {} back to {}", m.to, m.from));
        }
    }
    for file in &b.deletes {
        let path = Path::new(file);
        if !path.exists() {
            let bytes = std::fs::read(ctx.backup_of(path)?).map_err(|e| e.to_string())?;
            write_file(path, &bytes)?;
            changes.push(format!("restored {file}"));
        }
    }
    Ok(changes)
}
