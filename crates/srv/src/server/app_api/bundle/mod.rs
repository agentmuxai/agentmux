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

                let now = agentmux_common::time::now_ms();
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

pub(crate) async fn bundle_list_impl(state: &AppState) -> Result<serde_json::Value, String> {
    let memories = state.id_store.bundle_list().map_err(|e| format!("bundle.list: {e}"))?;
    let bundles: Vec<_> = memories.iter().map(|m| json!({
        "id": m.id, "name": m.name, "description": m.description,
        "provider": m.provider, "model": m.model, "is_blank": m.is_blank, "updated_at": m.updated_at,
    })).collect();
    // Emit both keys: `bundle.list` callers read `bundles`; the separate
    // REST route `/api/v1/agent/preset/list` (`PresetList` MCP tool,
    // server/mod.rs) still reads `presets` and is unrelated to the internal
    // WS `preset.*` aliases retired in this pass — do not drop `presets`
    // here without first retiring that REST route too.
    Ok(json!({ "bundles": bundles, "presets": bundles }))
}

pub(crate) async fn bundle_get_impl(
    state: &AppState,
    id: &str,
    name: &str,
) -> Result<serde_json::Value, String> {
    let memory = if !id.is_empty() {
        state.id_store.bundle_get(id).map_err(|e| format!("bundle.get: {e}"))?
            .ok_or_else(|| format!("bundle.get: not found id={id}"))?
    } else if !name.is_empty() {
        let all = state.id_store.bundle_list().map_err(|e| format!("bundle.get: {e}"))?;
        all.into_iter().filter(|m| m.name == name).max_by_key(|m| m.updated_at)
            .ok_or_else(|| format!("bundle.get: not found name={name}"))?
    } else {
        return Err("bundle.get: provide id or name".to_string());
    };
    serde_json::to_value(&memory).map_err(|e| e.to_string())
}

/// Structurally validate a bundle draft — Armory Bundle Format (ABF)
/// UI-alignment pass. Takes the SAME payload shape `bundle.upsert` accepts
/// (reuses its `normalize_bundle_upsert_input`), not just an id, so the
/// Armory editor's "Validate" button can check an unsaved draft (including a
/// brand-new bundle with no id yet) rather than only whatever was last
/// persisted.
///
/// Reads the bundle's bound MCP servers when the draft names a persisted
/// bundle. Before Phase 0b this was fully store-free and validated the inline
/// `mcp_servers` column; the ref tables are authoritative now, so validating
/// that column would report on data nothing else consumes
/// (`SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.4). An unsaved
/// draft has no bindings, so it is still checked without touching the store.
pub(crate) fn bundle_validate_impl(
    mstore: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    data: serde_json::Value,
) -> Result<crate::backend::bundle_validate::ValidationReport, String> {
    let memory: Bundle = serde_json::from_value(bundle::normalize_bundle_upsert_input(data))
        .map_err(|e| format!("bundle.validate: {e}"))?;
    let (mcp_entries, resolve_warnings) = if memory.id.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        // A store failure must NOT read as "this bundle has no components":
        // that would return a clean, apparently-successful report for a check
        // that never ran (Codex, PR #3153). The UI is built to show a failed
        // validate; give it one.
        let resolved = bundle::resolve_bundle_components(mstore, identity_store, &memory.id)
            .map_err(|e| format!("bundle.validate: {e}"))?;
        (resolved.mcp_entries, resolved.warnings)
    };
    let mut report = crate::backend::bundle_validate::validate_bundle(&memory, &mcp_entries);

    // The resolver drops a bound server whose config will not parse, and says
    // so in a warning. Discarding that left the validator reporting `is_valid`
    // for exactly the malformed component it exists to catch (Codex, PR
    // #3153) — the entry is absent from `mcp_entries`, so nothing downstream
    // could see it. Surface each as an error: unlike a duplicate name, an
    // unusable config is not a stylistic warning, it is a component that will
    // not load.
    for w in resolve_warnings {
        report.issues.push(crate::backend::bundle_validate::ValidationIssue {
            severity: crate::backend::bundle_validate::IssueSeverity::Error,
            field: "mcp_servers".to_string(),
            message: w,
        });
    }
    report.is_valid = !report
        .issues
        .iter()
        .any(|i| i.severity == crate::backend::bundle_validate::IssueSeverity::Error);

    Ok(report)
}

pub(crate) async fn bundle_self_get_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
) -> Result<serde_json::Value, String> {
    let agent_id = match owner.into() {
        SelfOwner::Slug(slug) => slug,
        // Identity M4c-2c: an attributed caller's own row, and its registry
        // record only by that row's slug AND id — never a same-named
        // agent's.
        SelfOwner::Uid(uid) => {
            let row = SelfOwner::caller_row(uid, &state.mstore)
                .map_err(|e| format!("bundle.self.get: {e}"))?;
            // The launch's bundle (`AgentInstance.memory_id`), as on the slug
            // path — not `AgentDefinition.memory_id`, the default a launch
            // inherits (`db_agents.default_memory_id`).
            let launch = state.mstore.instance_get(uid)
                .map_err(|e| format!("bundle.self.get: {e}"))?;
            let memory_id = launch
                .map(|i| i.memory_id)
                .filter(|m| !m.is_empty())
                .or_else(|| {
                    crate::backend::agent_registry_lookup::find_active_record_by_slug_and_definition(
                        &row.slug, &row.id,
                    )
                    .and_then(|rec| rec.data.memory_id)
                });
            return bundle_self_get_by_memory_id(state, memory_id);
        }
    };
    let instance = state.mstore.instance_get_by_slug(agent_id)
        .map_err(|e| format!("bundle.self.get: {e}"))?;
    // `instance_get_by_slug` only ever hits the local `db_agents` table — a
    // live agent that only exists in the global named-agent registry (never
    // created a `db_agents` instance row) falls through with `instance:
    // None` here. Without this fallback that silently read as "no bundle
    // bound" and returned the blank/vanilla preset regardless of what's
    // actually bound, with nothing to distinguish it from a genuinely
    // unbound agent. Mirrors the two-tier lookup
    // `native_memory_handlers::memory_dir_for_agent` already does for the
    // same reason (see that function's own doc comment, issue #1836).
    let memory_id = instance.as_ref()
        .and_then(|i| if i.memory_id.is_empty() { None } else { Some(i.memory_id.clone()) })
        .or_else(|| {
            crate::server::native_memory_handlers::find_active_registry_record_by_slug(agent_id)
                .and_then(|rec| rec.data.memory_id)
        });
    bundle_self_get_by_memory_id(state, memory_id)
}

/// The bundle `memory_id` names, or the blank singleton when none is bound.
fn bundle_self_get_by_memory_id(
    state: &AppState,
    memory_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let memory = if let Some(mid) = memory_id {
        state.id_store.bundle_get(&mid).map_err(|e| format!("bundle.self.get: {e}"))?
            .ok_or_else(|| format!("bundle.self.get: memory_id {mid} not found"))?
    } else {
        // No bundle bound: return the blank singleton (two-step — list to find
        // the blank id, then fetch the full object).
        let all = state.id_store.bundle_list().map_err(|e| format!("bundle.self.get: {e}"))?;
        let blank_id = all.into_iter().find(|m| m.is_blank).map(|m| m.id)
            .ok_or_else(|| "bundle.self.get: blank singleton not found".to_string())?;
        state.id_store.bundle_get(&blank_id).map_err(|e| format!("bundle.self.get: {e}"))?
            .ok_or_else(|| "bundle.self.get: blank singleton row missing".to_string())?
    };
    serde_json::to_value(&memory).map_err(|e| e.to_string())
}

/// The agent a self-scoped App API call is about — whose personal memory
/// (`memory.*`, M4c-2b), linked accounts (`identity.self.accounts`,
/// `identity.account.validate`), bound preset (`preset.get` self) and
/// history (`history.search`, M4c-2c) — identity spec §6.5.9.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SelfOwner<'a> {
    /// The calling agent's own row, by its token's UID. Versions are keyed by
    /// it as they are; the files are found by id
    /// (`memory_dir_for_agent_by_id`), and a UID with no row or no directory
    /// is an error — never another agent's directory (the #2901 class). A
    /// directory that resolves but does not exist still lists as empty, as
    /// on the slug path.
    Uid(&'a str),
    /// A slug, resolved as before M4c-2 (`memory_dir_for_agent`,
    /// `resolve_agent_id`, the registry by slug): an Unattributed HTTP
    /// caller, and the WS RPC.
    Slug(&'a str),
}

impl<'a> From<&'a str> for SelfOwner<'a> {
    fn from(slug: &'a str) -> Self {
        Self::Slug(slug)
    }
}

impl<'a> From<&'a String> for SelfOwner<'a> {
    fn from(slug: &'a String) -> Self {
        Self::Slug(slug)
    }
}

impl<'a> SelfOwner<'a> {
    /// The owner of an HTTP request: the caller's own row when attributed,
    /// whatever `agent_id` names — a name that is not the caller's is counted
    /// by M4a-2 — so a name two agents share no longer selects the other's
    /// memory, accounts or history. An Unattributed request keeps the slug, counted as
    /// `by_name_counter`.
    pub(crate) fn of(
        caller: Option<&'a crate::server::caller::Caller>,
        slug: &'a str,
        by_name_counter: &'static str,
    ) -> Self {
        match caller.and_then(crate::server::caller::Caller::uid) {
            Some(uid) => Self::Uid(uid),
            None => {
                crate::backend::agent_resolve::record_uid_fallback(by_name_counter);
                Self::Slug(slug)
            }
        }
    }

    /// The UID or the slug, for messages and logs.
    pub(super) fn label(self) -> &'a str {
        match self {
            Self::Uid(v) | Self::Slug(v) => v,
        }
    }

    /// The calling agent's row — a UID whose row is gone (a token that
    /// outlived its agent) is an error on every path, never an empty history
    /// or listing (ReAgent P1 on #3602).
    pub(crate) fn caller_row(
        uid: &str,
        mstore: &crate::backend::storage::store::Store,
    ) -> Result<crate::backend::storage::AgentDefinition, String> {
        mstore
            .agent_def_get(uid)
            .map_err(|e| format!("store: {e}"))?
            .ok_or_else(|| format!("calling agent {uid} not found"))
    }

    /// The owner's live memory directory.
    pub(super) fn dir(self, mstore: &crate::backend::storage::store::Store) -> Result<std::path::PathBuf, String> {
        match self {
            Self::Uid(uid) => {
                let agent = Self::caller_row(uid, mstore).map_err(|e| format!("memory: {e}"))?;
                crate::server::native_memory_handlers::memory_dir_for_agent_by_id(mstore, &agent)
                    .ok_or_else(|| format!("memory: memory directory for agent {uid} not found"))
            }
            Self::Slug(slug) => crate::server::native_memory_handlers::memory_dir_for_agent(mstore, slug),
        }
    }

    /// The owner's memory directory for a write: never an unverified guess
    /// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2).
    pub(super) fn dir_for_write(self, mstore: &crate::backend::storage::store::Store) -> Result<std::path::PathBuf, String> {
        let id = self.owner_id(mstore).map_err(|e| format!("memory: {e}"))?;
        match mstore.agent_def_get(&id).map_err(|e| format!("memory: store: {e}"))? {
            Some(agent) => crate::server::native_memory_handlers::memory_dir_for_write_by_id(mstore, &agent)
                .map_err(|e| format!("memory: {e}")),
            // No local row (a slug caller naming a registry-only agent): its
            // own spawn record still proves its directory.
            None => crate::server::native_memory_handlers::memory_dir_from_spawn(&id).ok_or_else(|| {
                format!(
                    "memory: agent {} has no local row and no spawn on record, so its memory directory \
                     can't be verified for a write",
                    self.label()
                )
            }),
        }
    }

    /// The owner's definition id (`db_agents.id`) — what its memory versions,
    /// mirror rows and identity links are keyed by.
    pub(super) fn owner_id(self, mstore: &crate::backend::storage::store::Store) -> Result<String, String> {
        match self {
            Self::Uid(uid) => Self::caller_row(uid, mstore).map(|row| row.id),
            Self::Slug(slug) => crate::server::native_memory_handlers::resolve_agent_uuid(mstore, slug),
        }
    }
}
