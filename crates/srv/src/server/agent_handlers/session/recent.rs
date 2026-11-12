// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `listrecentsessions`, moved out of `register` unchanged as named steps. It joins the agents the shared
//! registry knows with this channel's own instances, enriches each with its definition, account line, memory
//! and last activity, and answers the picker's "My Agents" list. The cascade follow-up (2026-05-23) added it;
//! the reattach itself is the existing continuation flow (`continueOfInstanceId` + `workDirOverride`, PR #977).
//!
//! Every data source degrades to empty on its own failure and is named in the reply's `degraded` list, so one
//! bad row cannot blank the whole list (the My Agents regression of 2026-07-27, PR #2296's oauth serde-tag
//! mismatch) and a caller can tell "no agents" from "a source failed".

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use super::super::account_label::{self, AccountLine};
use super::super::read_session_preview;
use super::definition_summary_attempted;
use crate::backend::agent_session::history_summary::{self, AgentHistory, HistoryRead};
use crate::backend::mps::Broker;
use crate::backend::rpc_types::{CommandListRecentSessionsData, ListRecentSessionsResult, RecentSessionRow};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::{AgentDefinition, AgentIdentityLink, AgentInstance, Bundle, IdentityAccount, Store};
use crate::registry::{NamedAgentRecord, NamedAgentRecordV1, Registry};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

/// The data sources that fell back to empty during one call, by name.
pub(super) type DegradedSources = Vec<&'static str>;

/// What the handler reads, cloned into the registered closure.
#[derive(Clone)]
pub(super) struct RecentSessionsDeps {
    pub mstore: Arc<Store>,
    pub id_store: Arc<Store>,
    pub identity_store: Arc<Store>,
    pub filestore: Arc<FileStore>,
    pub broker: Arc<Broker>,
}

/// The rows asked for (0 means the default, and the most is 100), and how many candidates to pull so that
/// filtering by snapshot and identity does not run out: ten times the rows, within 50 to 500.
pub(super) fn request_limits(requested: usize) -> (usize, usize) {
    let limit = if requested == 0 { DEFAULT_LIMIT } else { requested.min(MAX_LIMIT) };
    (limit, (limit * 10).max(50).min(500))
}

fn identity_filter(cmd: &CommandListRecentSessionsData) -> Option<&str> {
    cmd.identity_id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty())
}

pub(super) async fn list_recent_sessions(
    deps: &RecentSessionsDeps,
    data: serde_json::Value,
) -> Result<ListRecentSessionsResult, String> {
    let t0 = std::time::Instant::now();
    // A malformed request means the defaults.
    let cmd: CommandListRecentSessionsData = serde_json::from_value(data).unwrap_or_default();
    let (limit, raw_limit) = request_limits(cmd.limit);
    let mut degraded: DegradedSources = Vec::new();

    let instances = load_instances(&deps.mstore, raw_limit, identity_filter(&cmd), &mut degraded);
    let sources = load_sources(deps, &mut degraded);
    let account_names = account_label::names_by_id(&sources.accounts, &sources.mirror_accounts);
    let links_by_agent = group_links(&sources.links);
    let degraded_lookups = account_label::Degraded {
        links: degraded.contains(&"identity_links"),
        accounts: degraded.contains(&"accounts") && (sources.mirror_failed || sources.mirror_accounts.is_empty()),
    };

    // What each agent was last asked comes from its global transcript (keyed by agent id), not from this
    // channel's per-block snapshot, so a closed pane, an agent never launched here and a registry-only row all
    // get a real preview.
    let histories =
        history_summary::read_histories(instances.iter().map(|i| i.definition_id.clone()).collect()).await;
    if histories.values().any(|h| matches!(h, HistoryRead::Failed)) {
        degraded.push("transcript");
    }

    let mut unresolved_links = 0usize;
    let mut rows: Vec<RecentSessionRow> = Vec::with_capacity(instances.len());
    for inst in instances {
        let def = sources.defs.iter().find(|d| d.id == inst.definition_id);
        let line = account_line_for(&inst, def, &sources.defs, &links_by_agent, &account_names, degraded_lookups);
        unresolved_links += line.unresolved;
        let memory_name = memory_name(&inst, &sources.memories);
        let mut activity = activity_from_transcript(&inst, histories.get(&inst.definition_id), &deps.filestore)
            .unwrap_or_else(|| activity_from_snapshot(&inst, &deps.filestore, &mut degraded));
        fill_preview_from_summary(deps, &inst, def, &mut activity, &mut degraded);
        rows.push(build_row(&deps.mstore, &sources.defs, inst, def, line.text, memory_name, activity));
    }

    sort_recent_first(&mut rows);
    rows.truncate(limit);

    // The completion trace: whether the call ran, how long it took, and which sources fell back to empty.
    tracing::info!(
        rows = rows.len(),
        degraded = ?degraded,
        // Links to an account this store cannot find (deleted, or an unparseable secret_ref skipped by
        // identity_list): those agents show "No auth", and this says how many.
        unresolved_account_links = unresolved_links,
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "listrecentsessions: completed"
    );

    // `degraded` reaches the caller too: with every source degrading to empty instead of erroring the RPC, a
    // transport-level success no longer tells "genuinely zero agents" from "a source failed".
    Ok(ListRecentSessionsResult { rows, degraded: degraded.iter().map(|s| s.to_string()).collect() })
}

// ── Which agents ───────────────────────────────────────────────────────────

/// The agents to list. "My Agents" sources from the cross-version registry when there is one, so agents made in
/// any build, channel or version appear, not only this instance's local sessions; without a registry (CI, odd
/// environments) it is this channel's own instances.
fn load_instances(
    mstore: &Store,
    raw_limit: usize,
    identity_filter: Option<&str>,
    degraded: &mut DegradedSources,
) -> Vec<AgentInstance> {
    match mstore.shared_agent_registry() {
        Some(reg) => registry_instances(mstore, &reg, raw_limit, identity_filter, degraded),
        None => mstore.instance_list_named(raw_limit, None, identity_filter, true).unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                "listrecentsessions: local instance_list_named failed \
                 (no shared registry attached) — degrading to empty list"
            );
            degraded.push("local_instances");
            Vec::new()
        }),
    }
}

/// The registry's agents, each replaced by this channel's own instance of it when there is one, plus the local
/// agents no registry record represents (made before the live mirror could register them, or orphan
/// continuations whose head was deleted).
fn registry_instances(
    mstore: &Store,
    reg: &Registry,
    raw_limit: usize,
    identity_filter: Option<&str>,
    degraded: &mut DegradedSources,
) -> Vec<AgentInstance> {
    let agents_root = mstore.registry_agents_base();
    let records = reg.list_active().unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "listrecentsessions: registry list_active failed — \
             degrading to local-only instances for this call"
        );
        degraded.push("registry");
        Vec::new()
    });
    let records = newest_launch_per_agent(records, identity_filter, raw_limit);
    // Local agents in picker mode (include_continuations): each chain collapses to one row, and an orphan
    // continuation surfaces as its own root, so it does not vanish from "My Agents".
    let local = mstore.instance_list_named(raw_limit, None, identity_filter, true).unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "listrecentsessions: local instance_list_named failed — \
             degrading to registry-only instances for this call"
        );
        degraded.push("local_instances");
        Vec::new()
    });
    let local_by_key = newest_local_by_key(local);
    let mut out: Vec<AgentInstance> = records
        .into_iter()
        .map(|rec| overlay_or_synthesize(rec.data, &local_by_key, agents_root.as_deref()))
        .collect();
    append_local_only(&mut out, local_by_key);
    out
}

/// The records of the identity asked for (when one is), one per (definition, name) keeping the newest launch
/// (the registry read path lacks SQLite's chain-root collapse, so two fresh heads of one agent would double up),
/// newest first, at most `raw_limit`.
pub(super) fn newest_launch_per_agent(
    mut records: Vec<NamedAgentRecord>,
    identity_filter: Option<&str>,
    raw_limit: usize,
) -> Vec<NamedAgentRecord> {
    if let Some(idf) = identity_filter {
        records.retain(|r| r.data.identity_id.as_deref() == Some(idf));
    }
    records.sort_by(|a, b| {
        a.data
            .definition_id
            .cmp(&b.data.definition_id)
            .then_with(|| a.data.instance_name.cmp(&b.data.instance_name))
            .then_with(|| b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms))
    });
    records.dedup_by(|a, b| a.data.definition_id == b.data.definition_id && a.data.instance_name == b.data.instance_name);
    records.sort_by(|a, b| b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms));
    records.truncate(raw_limit);
    records
}

pub(super) type AgentKey = (String, String);

/// This channel's instances by (definition id, instance name), the same key the registry dedup uses, keeping the
/// newest, so the overlay and the local-only append agree on identity.
pub(super) fn newest_local_by_key(local: Vec<AgentInstance>) -> HashMap<AgentKey, AgentInstance> {
    let mut local_by_key: HashMap<AgentKey, AgentInstance> = HashMap::new();
    for li in local {
        let key = (li.definition_id.clone(), li.instance_name.clone());
        match local_by_key.get(&key) {
            Some(e) if e.started_at >= li.started_at => {}
            _ => {
                local_by_key.insert(key, li);
            }
        }
    }
    local_by_key
}

/// The local instance when this channel has one (the fresher source), else an instance made from the record.
fn overlay_or_synthesize(
    d: NamedAgentRecordV1,
    local_by_key: &HashMap<AgentKey, AgentInstance>,
    agents_root: Option<&Path>,
) -> AgentInstance {
    let key = (d.definition_id.clone(), d.instance_name.clone());
    match local_by_key.get(&key) {
        Some(li) => with_registry_session_id(li, d.session_id.as_deref()),
        None => synthesized_instance(d, agents_root),
    }
}

/// The local row, with the registry's session id when the local one is empty rather than an empty id the
/// picker's reattach would read as "start fresh" (SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
/// §4.1). The local row is normally fresher, but this holds for a row that predates the live write or raced it.
pub(super) fn with_registry_session_id(local: &AgentInstance, registry_session_id: Option<&str>) -> AgentInstance {
    if local.session_id.is_empty() {
        if let Some(sid) = registry_session_id.filter(|s| !s.is_empty()) {
            let mut merged = local.clone();
            merged.session_id = sid.to_string();
            return merged;
        }
    }
    local.clone()
}

/// A cross-channel row: no live block, and its workdir rebuilt from the record's source base (v3) or this
/// channel's agents base (legacy).
pub(super) fn synthesized_instance(d: NamedAgentRecordV1, agents_root: Option<&Path>) -> AgentInstance {
    let working_directory = match d.source_agents_base.as_deref() {
        Some(src) => Path::new(src).join(&d.working_dir).to_string_lossy().to_string(),
        None => match agents_root {
            Some(root) => root.join(&d.working_dir).to_string_lossy().to_string(),
            None => d.working_dir.clone(),
        },
    };
    AgentInstance {
        id: d.instance_id,
        definition_id: d.definition_id,
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: d.session_id.unwrap_or_default(),
        status: "available".to_string(),
        github_context: String::new(),
        started_at: d.last_launched_at_ms,
        ended_at: 0,
        created_at: d.created_at_ms,
        identity_id: d.identity_id.unwrap_or_default(),
        memory_id: d.memory_id.unwrap_or_default(),
        instance_name: d.instance_name,
        working_directory,
        display_hidden: false,
    }
}

/// The local agents whose (definition, name) no registry row represents, appended after the registry's.
pub(super) fn append_local_only(out: &mut Vec<AgentInstance>, local_by_key: HashMap<AgentKey, AgentInstance>) {
    let have_keys: HashSet<AgentKey> =
        out.iter().map(|i| (i.definition_id.clone(), i.instance_name.clone())).collect();
    for (key, li) in local_by_key {
        if !have_keys.contains(&key) {
            out.push(li);
        }
    }
}

// ── What each row shows ────────────────────────────────────────────────────

/// What the rows are enriched from, each read once for the whole call.
struct PickerSources {
    defs: Vec<AgentDefinition>,
    links: Vec<AgentIdentityLink>,
    accounts: Vec<IdentityAccount>,
    /// The global account mirror, read only when a link points at an account this channel's store does not hold
    /// (an isolated channel).
    mirror_accounts: Vec<IdentityAccount>,
    mirror_failed: bool,
    memories: Vec<Bundle>,
}

fn load_sources(deps: &RecentSessionsDeps, degraded: &mut DegradedSources) -> PickerSources {
    let defs = deps.mstore.agent_def_list().unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "listrecentsessions: agent_def_list failed — degrading to empty \
             (rows will show \"(missing definition)\")"
        );
        degraded.push("agent_defs");
        Vec::new()
    });
    // Identity names resolve off the direct agent<->account links (db_agent_identity_links, db_accounts), bulk
    // fetched once and grouped by agent rather than queried per row.
    let links = deps.identity_store.agent_identity_list_all().unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "listrecentsessions: agent_identity_list_all failed — degrading to \
             empty (rows will show no account line)"
        );
        degraded.push("identity_links");
        Vec::new()
    });
    let accounts = deps.id_store.identity_list(None).unwrap_or_else(|e| {
        // The call PR #2296 broke: it now degrades in isolation instead of aborting the handler.
        tracing::warn!(
            error = %e,
            "listrecentsessions: identity_list failed — degrading to empty \
             (the global mirror fills in what it can; a link it cannot resolve shows no account line)"
        );
        degraded.push("accounts");
        Vec::new()
    });
    let own_ids: HashSet<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
    let mut mirror_failed = false;
    let mirror_accounts: Vec<IdentityAccount> = if links.iter().any(|l| !own_ids.contains(l.account_id.as_str())) {
        deps.identity_store.identity_list(None).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "listrecentsessions: the global account mirror could not be read");
            mirror_failed = true;
            Vec::new()
        })
    } else {
        Vec::new()
    };
    let memories = deps.id_store.bundle_list().unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "listrecentsessions: bundle_list failed — degrading to empty \
             (rows will show \"(missing memory)\")"
        );
        degraded.push("memories");
        Vec::new()
    });
    PickerSources { defs, links, accounts, mirror_accounts, mirror_failed, memories }
}

pub(super) fn group_links(links: &[AgentIdentityLink]) -> HashMap<&str, Vec<&AgentIdentityLink>> {
    let mut by_agent: HashMap<&str, Vec<&AgentIdentityLink>> = HashMap::new();
    for link in links {
        by_agent.entry(link.agent_id.as_str()).or_default().push(link);
    }
    by_agent
}

/// The agent's own account links, falling back to its template's. A launch-only agent can hold a link of its
/// own (the FK targets `db_agents` since Phase 3c, #3088), but nothing writes one there yet: the only binding UI
/// targets a template or user agent's own definition id. Until a launch-scoped binding path exists, dropping the
/// fallback would make a launched agent's identity disappear (retire it with Phase 3d).
fn account_line_for(
    inst: &AgentInstance,
    def: Option<&AgentDefinition>,
    defs: &[AgentDefinition],
    links_by_agent: &HashMap<&str, Vec<&AgentIdentityLink>>,
    account_names: &HashMap<&str, &str>,
    degraded_lookups: account_label::Degraded,
) -> AccountLine {
    // The template it came from (empty `parent_id` for an agent never cloned from one), used only here.
    let template = def.filter(|d| !d.parent_id.is_empty()).and_then(|d| defs.iter().find(|t| t.id == d.parent_id));
    let links_key = match links_by_agent.get(inst.definition_id.as_str()) {
        Some(_) => inst.definition_id.as_str(),
        None => template.map(|t| t.id.as_str()).unwrap_or(inst.definition_id.as_str()),
    };
    account_label::account_line(links_by_agent.get(links_key).map(|v| v.as_slice()), account_names, degraded_lookups)
}

pub(super) fn memory_name(inst: &AgentInstance, memories: &[Bundle]) -> String {
    if inst.memory_id.is_empty() {
        return "(vanilla CLI)".to_string();
    }
    memories
        .iter()
        .find(|m| m.id == inst.memory_id)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| "(missing memory)".to_string())
}

// ── Last activity ──────────────────────────────────────────────────────────

/// When an agent was last active and what it was last asked: from its transcript, else this channel's snapshot.
pub(super) struct Activity {
    pub(super) has_snapshot: bool,
    pub(super) last_active_at: i64,
    pub(super) preview: String,
    pub(super) node_count: usize,
    pub(super) snapshot_check_failed: bool,
}

impl Activity {
    /// Nothing to show: the launch time stands in for the last activity.
    fn none(started_at: i64) -> Self {
        Activity { has_snapshot: false, last_active_at: started_at, preview: String::new(), node_count: 0, snapshot_check_failed: false }
    }

    /// The read failed, which the row reports instead of looking like an agent with no history.
    fn check_failed(started_at: i64) -> Self {
        Activity { snapshot_check_failed: true, ..Self::none(started_at) }
    }
}

/// `node_count` is 0: counting messages would mean reading the whole transcript, so the tile shows none rather
/// than a guess.
fn transcript_activity(h: &AgentHistory, started_at: i64) -> Activity {
    Activity {
        has_snapshot: true,
        last_active_at: if h.last_activity_ms > 0 { h.last_activity_ms } else { started_at },
        preview: h.last_user_message.clone(),
        node_count: 0,
        snapshot_check_failed: false,
    }
}

/// The agent's global transcript, keyed by its id. None when there is none to use, so the per-block snapshot,
/// which old builds wrote, answers instead.
pub(super) fn activity_from_transcript(inst: &AgentInstance, read: Option<&HistoryRead>, filestore: &FileStore) -> Option<Activity> {
    match read {
        Some(HistoryRead::Transcript(h)) => Some(transcript_activity(h, inst.started_at)),
        Some(HistoryRead::Failed) => Some(Activity::check_failed(inst.started_at)),
        // The agent's zone in the shared store is corrupt (`transcript::damage`): this channel's own copy of the
        // conversation, kept under the pane's block id, answers.
        Some(HistoryRead::Damaged) if inst.block_id.is_empty() => None,
        Some(HistoryRead::Damaged) => match history_summary::read_block_history(filestore, &inst.block_id) {
            HistoryRead::Transcript(h) => Some(transcript_activity(&h, inst.started_at)),
            HistoryRead::NoTranscript => None,
            HistoryRead::Failed | HistoryRead::Damaged => Some(Activity::check_failed(inst.started_at)),
        },
        Some(HistoryRead::NoTranscript) | None => None,
    }
}

/// This channel's per-block snapshot: a stat first (cheap, and it gives the sort time), the content only when it
/// exists. `Ok(None)` (the block never wrote one) and `Err` (a filestore I/O error) are different rows: a real
/// error on an agent that does have history must not read as "never had one"
/// (REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §5).
pub(super) fn activity_from_snapshot(inst: &AgentInstance, filestore: &FileStore, degraded: &mut DegradedSources) -> Activity {
    if inst.block_id.is_empty() {
        return Activity::none(inst.started_at);
    }
    match filestore.stat(&inst.block_id, "output.state.json") {
        Ok(Some(file)) => {
            let modts = if file.modts > 0 { file.modts } else { inst.started_at };
            let (preview, node_count) = read_session_preview(filestore, &inst.block_id);
            Activity { has_snapshot: true, last_active_at: modts, preview, node_count, snapshot_check_failed: false }
        }
        Ok(None) => Activity::none(inst.started_at),
        Err(e) => {
            tracing::warn!(
                error = %e,
                definition_id = %inst.definition_id,
                block_id = %inst.block_id,
                "listrecentsessions: filestore.stat(output.state.json) failed — \
                 row will report snapshot_check_failed instead of silently \
                 looking like it never had a snapshot"
            );
            if !degraded.contains(&"snapshot_stat") {
                degraded.push("snapshot_stat");
            }
            Activity::check_failed(inst.started_at)
        }
    }
}

/// No structured snapshot to preview: use the agent's persisted summary, or ask for one once per definition per
/// process (a Haiku summary of its raw terminal output, REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT
/// §5a). Only for a local row with a real block: a cross-channel row has no local raw output, and keeps the
/// plain fallback text, which is accurate for it.
fn fill_preview_from_summary(
    deps: &RecentSessionsDeps,
    inst: &AgentInstance,
    def: Option<&AgentDefinition>,
    activity: &mut Activity,
    degraded: &mut DegradedSources,
) {
    if activity.has_snapshot || inst.block_id.is_empty() {
        return;
    }
    match deps.mstore.agent_activity_summary_get(&inst.definition_id) {
        Ok(Some(existing)) if !existing.summary.is_empty() => {
            activity.preview = existing.summary;
        }
        // `insert` is true only the first time a definition is claimed in this process (as
        // `SubagentWatcher.naming_triggered` does, #2786): the ambient gateway's `admit` only
        // coalesces concurrent triggers, so without it a definition that can never be summarized would be
        // re-spawned on every later call. A successful summary persists and is found by the arm above. A stored
        // row with an empty summary is treated as none.
        Ok(None) | Ok(Some(_)) if definition_summary_attempted().lock().unwrap().insert(inst.definition_id.clone()) => {
            spawn_definition_summary(deps, inst, def);
        }
        Ok(None) | Ok(Some(_)) => {}
        Err(e) => {
            tracing::warn!(
                error = %e,
                definition_id = %inst.definition_id,
                "listrecentsessions: agent_activity_summary_get failed — \
                 row keeps its plain fallback preview text"
            );
            if !degraded.contains(&"activity_summary") {
                degraded.push("activity_summary");
            }
        }
    }
}

/// Fire and forget: a real Haiku CLI round trip must not block the response. Its spend is logged and no more:
/// no UI total exists for any background ambient call (#2786).
fn spawn_definition_summary(deps: &RecentSessionsDeps, inst: &AgentInstance, def: Option<&AgentDefinition>) {
    let mstore = deps.mstore.clone();
    let filestore = deps.filestore.clone();
    let broker = deps.broker.clone();
    let definition_id = inst.definition_id.clone();
    let block_id = inst.block_id.clone();
    let provider_id = def.map(|d| d.provider.clone()).unwrap_or_default();
    tokio::spawn(async move {
        let result = crate::ambient::tasks::generate_definition_activity_summary(
            &mstore,
            &filestore,
            &broker,
            &definition_id,
            &block_id,
            &provider_id,
        )
        .await;
        if let Some(generated) = result {
            if let Some(tokens) = generated.tokens {
                tracing::debug!(
                    definition_id = %definition_id,
                    ?tokens,
                    "listrecentsessions: definition activity summary generated"
                );
            }
        }
    });
}

// ── The row, and the order ─────────────────────────────────────────────────

fn build_row(
    mstore: &Store,
    defs: &[AgentDefinition],
    inst: AgentInstance,
    def: Option<&AgentDefinition>,
    identity_name: String,
    memory_name: String,
    activity: Activity,
) -> RecentSessionRow {
    let seat = crate::backend::agent_seat::summary_in(mstore, defs, &inst.definition_id);
    RecentSessionRow {
        instance_id: inst.id,
        instance_name: inst.instance_name,
        definition_id: inst.definition_id.clone(),
        definition_name: def.map(|d| d.name.clone()).unwrap_or_else(|| "(missing definition)".to_string()),
        provider: def.map(|d| d.provider.clone()).unwrap_or_default(),
        model_vendor_base_url: def.map(|d| d.model_vendor_base_url.clone()).unwrap_or_default(),
        working_directory: inst.working_directory,
        identity_id: inst.identity_id,
        identity_name,
        memory_id: inst.memory_id,
        memory_name,
        block_id_hint: inst.block_id,
        // The CLI-captured session id, so the picker's reattach can `--resume <sid>` on the first turn of the new
        // block; without it the new subprocess starts a fresh session and the CLI re-injects the startup context.
        session_id: inst.session_id,
        preview: activity.preview,
        node_count: activity.node_count,
        last_active_at: activity.last_active_at,
        has_snapshot: activity.has_snapshot,
        snapshot_check_failed: activity.snapshot_check_failed,
        agent_created_at: def.map(|d| d.created_at).unwrap_or(0),
        started_at: inst.started_at,
        agent_type: def.map(|d| d.agent_type.clone()).unwrap_or_default(),
        seat,
    }
}

/// Rows with a snapshot first (newest activity first), then the rest by launch time, so live conversations stay
/// on top while legacy rows still surface.
pub(super) fn sort_recent_first(rows: &mut [RecentSessionRow]) {
    rows.sort_by(|a, b| match (a.has_snapshot, b.has_snapshot) {
        (true, true) | (false, false) => b.last_active_at.cmp(&a.last_active_at),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
    });
}
