// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.*` App API handlers — the Armory Bundle Format (ABF) surface.
//!
//! Module layout (split 2026-09-22): this file holds registration, the plain
//! CRUD/validate/self_get handlers, and the helpers every family shares;
//! each `bundle.*` family lives in its own child module. The children are
//! glob-imported back here so that, exactly as when this was one file, any
//! function can call any other without an import.

use super::*;

/// Raw `[{path, content}]` entry shape, shared by `bundle.import`'s `files`
/// input and the Phase 3 `bundle.import.preview`/`.commit` handlers. Now one
/// definition in rpc_types rather than a private copy here, so the generated
/// bindings and this file cannot disagree about it.
use crate::backend::rpc_types::BundleImportFileEntry as FileEntry;

// `PreviewReq`/`CommitReq`/`SkillSelection` moved to
// backend::rpc_types::bundle_import so the bindings generator can see them --
// they were private to this file, which is why the frontend's copies were
// hand-written and, in preview's case, wrong (it declared only `file_path`).
use crate::backend::rpc_types::{
    BundleImportSkillSelection as SkillSelection,
    CommandBundleImportCommitData as CommitReq,
    CommandBundleImportPreviewData as PreviewReq,
};

mod components;
mod export;
mod export_for_agent;
mod import;
mod import_for_agent;

#[cfg(test)]
mod tests;

use components::*;
use export::*;
use export_for_agent::*;
use import::*;
use import_for_agent::*;

// Referenced from outside `bundle` under these paths and visibilities.
pub(crate) use components::purge_bundle_component_refs;
pub(super) use components::resolve_bundle_components;
pub(crate) use components::MEMORY_NOT_EXPORTED_WARNING;

/// Per-agent async lock serializing `bundle.import_for_agent`'s "check
/// zero existing memory rows, then write" sequence — ABF v0.2 §2.3,
/// reagent P2 on PR #2527. Mirrors `agent_open.rs`'s `AGENT_OPEN_LOCKS`
/// precedent exactly, including its scope note: this only serializes
/// calls handled by THIS process, not a genuinely different AgentMux
/// instance/channel racing the same agent_id.
static BUNDLE_IMPORT_FOR_AGENT_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn bundle_import_for_agent_lock(agent_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = BUNDLE_IMPORT_FOR_AGENT_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks
        .entry(agent_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_bundle_list(engine, state);
    register_bundle_get(engine, state);
    register_bundle_upsert(engine, state);
    register_bundle_delete(engine, state);
    register_bundle_self_get(engine, state);
    register_bundle_export(engine, state);
    register_bundle_import(engine, state);
    register_bundle_import_preview(engine, state);
    register_bundle_import_commit(engine, state);
    register_bundle_export_for_agent(engine, state);
    register_bundle_export_for_agent_with_history(engine, state);
    register_bundle_import_for_agent(engine, state);
    register_bundle_validate(engine, state);
    register_agent_project_instructions(engine, state);
}

/// Normalize a `bundle.upsert` request body into the shape the `Bundle` struct
/// deserializes from, so the App API accepts the request exactly as documented
/// in the spec:
///   - `id` may be omitted to create (the struct has no serde default), so an
///     absent or null `id` is filled with an empty string (the handler then
///     mints a UUID).
///   - `context_files` / `mcp_servers` / `skills` are JSON-encoded array strings
///     on the struct, but the spec shows them as JSON arrays. Array values are
///     re-encoded to their JSON string form; values already given as strings
///     pass through untouched.
pub(super) fn normalize_bundle_upsert_input(mut data: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = data {
        match map.get("id") {
            Some(serde_json::Value::String(_)) => {}
            _ => {
                map.insert("id".to_string(), serde_json::Value::String(String::new()));
            }
        }
        for key in ["context_files", "mcp_servers", "skills"] {
            if let Some(v) = map.get(key) {
                if v.is_array() {
                    let encoded =
                        serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
                    map.insert(key.to_string(), serde_json::Value::String(encoded));
                }
            }
        }
    }
    data
}

// The deprecated `preset.*` aliases (Phase 2's one-release compat window)
// were retired as part of the Armory Bundle Format (ABF) UI-alignment pass —
// `bundle.*` is the only surface now; see the ABF branding sweep this
// change shipped alongside.

fn register_bundle_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |_data, _ctx| {
            let state = state.clone();
            Box::pin(async move { Ok(Some(bundle_list_impl(&state).await?)) })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_LIST, make(state.clone()));
}

fn register_bundle_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    #[serde(default)] id: String,
                    #[serde(default)] name: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.get: {e}"))?;
                Ok(Some(bundle_get_impl(&state, &req.id, &req.name).await?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_GET, make(state.clone()));
}

// NOT migrated to `register_typed`, deliberately, unlike the rest of the
// bundle surface. `bundle_validate_impl` runs its payload through
// `normalize_bundle_upsert_input` BEFORE deserializing it -- that is what
// fills a missing `id` with "" so an unsaved draft can be validated, and what
// coerces array-valued `context_files`/`mcp_servers`/`skills` into the
// JSON-encoded strings `Bundle` expects.
//
// `register_typed` deserializes the payload into `Req` first and hands the
// handler a typed value, so there is no point at which that normalize step
// could run. Typing this command would mean re-implementing normalize as
// custom serde deserializers -- duplicating security-relevant coercion logic
// to satisfy the generator, which is the wrong trade. The RESPONSE is typed
// (`ValidationReport`, ts-rs-generated) and that is where the drift risk
// actually was; the request stays a `Value` on purpose.
fn register_bundle_validate(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let handler: crate::backend::rpc::engine::CommandHandler = Box::new(move |data, _ctx| {
        let mstore = mstore.clone();
        let identity_store = identity_store.clone();
        Box::pin(async move {
            let report = bundle_validate_impl(&mstore, &identity_store, data)?;
            Ok(Some(serde_json::to_value(&report).map_err(|e| e.to_string())?))
        })
    });
    engine.register_handler(COMMAND_BUNDLE_VALIDATE, handler);
}

/// Reject a `bundle.upsert` write that would change `provider`/`model` on
/// an existing bundle that already has them set —
/// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.2. These are
/// readonly-once-set: an ABF's portability guarantee (it's self-describing
/// about what it needs to run) depends on them never silently changing
/// after creation, and the UI-only disabling in `memory-manager.tsx` is
/// advisory, not a guarantee — this is the actual enforcement.
///
/// `existing = None` (fresh insert) or an existing row with `provider`/
/// `model` still empty (legacy row awaiting backfill, or was raced to
/// create without them) always allows the write — that's how a bundle's
/// provider/model get set the FIRST time. Once non-empty, they're locked.
///
/// Pure — no I/O — directly unit-testable without spinning up an
/// `AppState`, mirroring `agent_open.rs`'s `resolve_vendor_env_override`.
fn check_provider_model_immutable(existing: Option<&Bundle>, incoming: &Bundle) -> Result<(), String> {
    let Some(existing) = existing else { return Ok(()) };
    if !existing.provider.is_empty() && existing.provider != incoming.provider {
        return Err(format!(
            "FORBIDDEN: bundle {} provider is readonly once set (has '{}', got '{}')",
            existing.id, existing.provider, incoming.provider
        ));
    }
    if !existing.model.is_empty() && existing.model != incoming.model {
        return Err(format!(
            "FORBIDDEN: bundle {} model is readonly once set (has '{}', got '{}')",
            existing.id, existing.model, incoming.model
        ));
    }
    Ok(())
}

fn register_bundle_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Bundle =
                    serde_json::from_value(normalize_bundle_upsert_input(data))
                        .map_err(|e| format!("bundle.upsert: {e}"))?;

                // S4: guard on the target id, not the caller-supplied is_blank flag
                // (which defaults false and can be omitted to bypass is_blank check).
                if memory.id == "blank" || memory.id.starts_with("seed-") || memory.is_blank {
                    return Err("FORBIDDEN: cannot mutate a protected bundle".to_string());
                }
                // Guard existing global bundles: an agent must not be able to demote or
                // corrupt a shared global bundle it doesn't own by supplying its id.
                if !memory.id.is_empty() {
                    if let Some(existing) = id_store.bundle_get(&memory.id)
                        .map_err(|e| format!("bundle.upsert: {e}"))?
                    {
                        if existing.is_global {
                            return Err("FORBIDDEN: cannot mutate a global bundle".to_string());
                        }
                        // provider/model are readonly once set — the actual
                        // enforcement behind the portability guarantee (the
                        // bundle editor's disabled-input UI is advisory
                        // only). See check_provider_model_immutable's doc
                        // comment.
                        check_provider_model_immutable(Some(&existing), &memory)?;
                    }
                }
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                // S4a: strip caller-supplied escalation fields.
                memory.is_global = false;
                memory.sort_order = 0;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 { memory.created_at = now; }
                memory.updated_at = now;

                id_store.bundle_upsert(&memory)
                    .map_err(|e| format!("bundle.upsert: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&memory).map_err(|e| e.to_string())?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_UPSERT, make(state));
}

fn register_bundle_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let mstore = state.mstore.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let mstore = mstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.delete: {e}"))?;

                if req.id == "blank" {
                    return Err("FORBIDDEN: cannot delete a seeded bundle".to_string());
                }

                match id_store.bundle_delete(&req.id) {
                    Ok(deleted) => {
                        if deleted {
                            purge_bundle_component_refs(&mstore, &req.id);
                            broker.publish(crate::backend::mps::MuxEvent {
                                event: "memories:changed".to_string(),
                                scopes: vec![], sender: String::new(), persist: 0, data: None,
                            });
                        }
                        Ok(Some(json!({ "deleted": deleted })))
                    }
                    Err(crate::backend::storage::error::StoreError::Other(msg))
                        if msg.contains("seed") || msg.contains("seeded") =>
                    {
                        Err(format!("FORBIDDEN: cannot delete a seeded bundle"))
                    }
                    Err(e) => Err(format!("bundle.delete: {e}")),
                }
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_DELETE, make(state));
}

fn register_bundle_self_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.self.get: {e}"))?;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;
                Ok(Some(bundle_self_get_impl(&state, &req.agent_id).await?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_SELF_GET, make(state.clone()));
}

/// Resolve the agent definition behind an S1-authenticated `agent_id`.
///
/// **`check_s1` authenticates a SLUG, not a UUID.** `RpcContext.agent_id` is
/// "slug of the authenticated agent from bus:register"
/// (`rpc_types/misc.rs:221`), while `agent_def_get` queries `db_agents` by its
/// UUID primary key — so calling it with the authenticated value returns
/// `None` for every real caller (ReAgent P0, PR #3156).
///
/// Delegates to `resolve_agent_definition_id`, the existing resolver for
/// exactly this, rather than hand-rolling the lookup: it already carries the
/// registry-only fallback a previous review round forced in, for a live agent
/// that never created a local `db_agents` row. A fresh two-tier lookup here
/// would have that hole again.
fn resolve_agent_for_s1(
    state: &AppState,
    agent_id: &str,
) -> Result<crate::backend::storage::AgentDefinition, String> {
    let def_id = resolve_agent_definition_id(state, agent_id)?;
    state
        .mstore
        .agent_def_get(&def_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no agent with slug or id {agent_id}"))
}

/// `agent.project_instructions` — what this agent will actually read.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Until
/// now nothing could answer this: AgentMux knew what it wrote and recorded a
/// single boolean about anything it found already there (§2.4).
///
/// Agent-scoped, so `check_s1` applies — an agent's working directory contents
/// are not window-scoped data the way a bundle is.
///
/// **Read-only, and there is no write counterpart on purpose.** A foreign
/// instruction file belongs to the repository;
/// `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to keep AgentMux
/// out of it, and this must not become a way around that.
fn register_agent_project_instructions(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req {
                    agent_id: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("agent.project_instructions: {e}"))?;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;

                let agent = resolve_agent_for_s1(&state, &req.agent_id)
                    .map_err(|e| format!("agent.project_instructions: {e}"))?;

                // The directory the agent will actually run in: a blank
                // working_directory means the per-agent default and a `~`
                // path is expanded at launch, so the stored value alone
                // describes a directory the agent never uses (Codex, #3156).
                let work_dir = crate::backend::project_instructions::effective_working_dir(
                    &agent.working_directory,
                    &agent.name,
                );
                let files = crate::backend::project_instructions::resolve_project_instructions(
                    &agent.provider,
                    &work_dir,
                );

                // Compare against what was recorded at the last launch, so a
                // repository file that changed underneath is visible rather
                // than merely present. Observations are written at
                // `agent.open` (`observe_project_instructions`) — this RPC
                // stays a pure read, which is why a file edited since the last
                // launch reports `modified` every time it is called until the
                // agent is next opened.
                let previous = state
                    .mstore
                    .project_instructions_list(&agent.id)
                    .unwrap_or_default();
                let mut enriched: Vec<serde_json::Value> = files
                    .iter()
                    .map(|f| {
                        let prev = previous.iter().find(|p| p.path == f.path);
                        let change = crate::backend::storage::project_instructions::classify_change(
                            prev,
                            f.exists,
                            &f.content_hash,
                        );
                        let mut v = serde_json::to_value(f).unwrap_or_default();
                        if let Some(obj) = v.as_object_mut() {
                            obj.insert("change".to_string(), json!(change));
                            obj.insert(
                                "last_observed_at".to_string(),
                                json!(prev.map(|p| p.observed_at)),
                            );
                        }
                        v
                    })
                    .collect();

                // A path the resolver no longer returns at all — a scanned
                // `.github/instructions/*.instructions.md` deleted since the
                // last launch — would otherwise just vanish from the response,
                // never reported as `removed`, and recreating it later would
                // read as `unchanged` against the stale row (Codex, PR #3162).
                // The declared paths always come back (absent ones included),
                // so this only ever fires for dynamically scanned files.
                for prev in previous.iter().filter(|p| !files.iter().any(|f| f.path == p.path)) {
                    let change = crate::backend::storage::project_instructions::classify_change(
                        Some(prev),
                        false,
                        "",
                    );
                    enriched.push(json!({
                        "path": prev.path,
                        "exists": false,
                        "size_bytes": 0,
                        "content_hash": "",
                        "owner": prev.owner,
                        "content": "",
                        "truncated": false,
                        "error": serde_json::Value::Null,
                        "change": change,
                        "last_observed_at": prev.observed_at,
                    }));
                }

                Ok(Some(json!({
                    "provider": agent.provider,
                    "working_directory": work_dir,
                    "files": enriched,
                })))
            })
        })
    };
    engine.register_handler(COMMAND_AGENT_PROJECT_INSTRUCTIONS, make(state.clone()));
}

/// Read-only account-requirement resolution (spec §4.5) — one identity
/// lookup per DISTINCT provider, not per requirement row (codex P1, PR
/// #2379 round 4: `bundle_import.rs` bounds the requirements array length,
/// but even at that bound many rows commonly share a handful of
/// providers). Extracted here (Phase 3 spec §3.1's account-resolution
/// implementation note) so the existing `bundle.import` route and the new
/// `bundle.import.preview`/`.commit` handlers share the exact same
/// resolution logic rather than duplicating it in a second place where it
/// could drift.
/// Full, untruncated values — callers apply the shared bounded-display
/// projection ([`bounded_display`]) to `id`/`provider`/`env` before ever
/// serializing these into an RPC response (Phase 3 spec §3.1, round 11).
struct RequirementResolution {
    id: String,
    provider: String,
    env: String,
    match_count: usize,
}

impl RequirementResolution {
    fn resolved(&self) -> bool {
        self.match_count == 1
    }
}

fn resolve_account_requirements(
    id_store: &crate::backend::storage::store::Store,
    requirements: &[crate::backend::bundle_import::AccountRequirement],
) -> Result<Vec<RequirementResolution>, String> {
    let mut results: Vec<RequirementResolution> = Vec::new();
    let mut match_count_by_provider: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for requirement in requirements {
        let match_count = match match_count_by_provider.get(requirement.provider.as_str()) {
            Some(&n) => n,
            None => {
                // Codex P2, PR #2379 round 2: account CRUD routes through
                // id_store (shared/store.db when available), not mstore
                // (the per-channel database).
                let n = id_store
                    .identity_list(Some(&requirement.provider))
                    .map_err(|e| format!("account lookup failed: {e}"))?
                    .len();
                match_count_by_provider.insert(requirement.provider.as_str(), n);
                n
            }
        };
        results.push(RequirementResolution {
            id: requirement.id.clone(),
            provider: requirement.provider.clone(),
            env: requirement.env.clone(),
            match_count,
        });
    }
    Ok(results)
}

/// Shared bounded-display projection (Phase 3 spec, round 12's governing
/// rule) for every "meant to be short" field both `preview` and `commit`
/// echo back: skill slug/description, requirement `id`/`provider`/`env`,
/// context-file `display_path`, bundle `description`, and commit's
/// `skipped_skills`/`resolved_requirement_ids` entries. `bundle.name` is
/// NOT projected through this — it's bounded at parse time instead
/// (round 13), since it's re-submitted verbatim as `bundle_name`.
fn bounded_display(s: &str) -> String {
    crate::backend::bundle_import::truncate_display(s, crate::backend::bundle_import::MAX_DISPLAY_FIELD_CHARS)
}
