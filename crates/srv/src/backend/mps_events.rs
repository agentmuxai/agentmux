// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Broker events with a typed payload (docs/specs/SPEC_TYPED_EVENT_PAYLOADS_2026_10_13.md).
//!
//! Each event is a marker type naming its wire name and its payload type, declared once in the
//! `events!` list below. That list is also the schema: `schema()` (exported to
//! `frontend/types/rpc/event-schema.json`) feeds the generated `MuxEventPayloads` map, so the
//! frontend's `onMuxEvent(WpsEvent.X, (data) => …)` gets `data` as the Rust type. Publish one with
//! [`Broker::publish_event`](super::mps::Broker::publish_event). An event not listed here is still
//! published as a plain `MuxEvent` with untyped data.

use serde::Serialize;

#[cfg(test)]
use crate::backend::rpc::schema::{ts_type, TsType};

/// A broker event with a typed payload.
pub trait Event {
    /// The wire name (`MuxEvent::event`).
    const NAME: &'static str;
    type Payload: Serialize + ts_rs::TS + 'static;
}

macro_rules! events {
    ($($(#[$doc:meta])* $marker:ident: $name:expr => $payload:ty,)*) => {
        $(
            $(#[$doc])*
            pub struct $marker;
            impl Event for $marker {
                const NAME: &'static str = $name;
                type Payload = $payload;
            }
        )*
        /// Every typed event: its wire name and its payload as TypeScript (the export test reads it).
        #[cfg(test)]
        pub fn schema() -> Vec<(&'static str, TsType)> {
            vec![$(($name, ts_type::<$payload>())),*]
        }
    };
}

events! {
    /// A block controller's runtime status (`publish_controller_status`).
    ControllerStatus: super::mps::EVENT_CONTROLLER_STATUS => crate::backend::blockcontroller::BlockControllerRuntimeStatus,
    /// One sysinfo sample for a connection (`sysinfo::run_sysinfo_loop`).
    SysInfo: super::mps::EVENT_SYS_INFO => crate::backend::rpc_types::TimeSeriesData,
    /// An agent's turn ledger changed (`blockcontroller::health::ledger_publisher`).
    AgentTurn: super::mps::EVENT_AGENT_TURN => crate::backend::blockcontroller::health::AgentTurnEvent,
    /// An agent failed, classified (`agents::failure`).
    AgentFailed: super::mps::EVENT_AGENT_FAILURE => crate::agents::failure::AgentFailure,
    /// What an agent process runs with, and whether a restart is pending.
    AgentRuntime: super::mps::EVENT_AGENT_RUNTIME => crate::backend::agent_runtime::AgentRuntimeEvent,
    /// Where an agent's CLI auto-compacts.
    AgentContextUsageChanged: super::mps::EVENT_AGENT_CONTEXT_USAGE => crate::backend::agent_context_usage::AgentContextUsageEvent,
    /// A queued message reached the agent.
    AgentMessageAccepted: super::mps::EVENT_AGENT_MESSAGE_ACCEPTED => crate::backend::blockcontroller::MessageAcceptedEvent,
    /// A stale `--resume` is being retried, or the retry resolved.
    AgentResumeRetry: super::mps::EVENT_AGENT_RESUME_RETRY => ResumeRetryEvent,
    /// One line of a closing agent pane's shutdown log (`sagas::close_pane::publish_shutdown`).
    AgentShutdown: crate::sagas::close_pane::EVENT_AGENT_SHUTDOWN => crate::sagas::close_pane::ShutdownLineEvent,
    /// A call to action srv holds open started, or ended (`backend::user_attention`).
    UserAttention: super::mps::EVENT_USER_ATTENTION => crate::backend::user_attention::UserAttentionEvent,
    /// New events from one subagent (`subagent_watcher`).
    SubagentActivity: "subagent:activity" => crate::backend::subagent_watcher::events::SubagentActivityEvent,
    /// A workflow dispatch's coalesced new events.
    DispatchActivity: "dispatch:activity" => crate::backend::subagent_watcher::events::DispatchActivityEvent,
    SubagentSpawned: "subagent:spawned" => crate::backend::subagent_watcher::events::SubagentSpawnedEvent,
    SubagentCompleted: "subagent:completed" => crate::backend::subagent_watcher::events::SubagentCompletedEvent,
    SubagentNamed: "subagent:named" => crate::backend::subagent_watcher::events::SubagentNamedEvent,
    SubagentUpdated: "subagent:updated" => crate::backend::subagent_watcher::events::SubagentUpdatedEvent,
    DispatchUpdated: "dispatch:updated" => crate::backend::subagent_watcher::events::DispatchUpdatedEvent,
    SubagentAbandoned: "subagent:abandoned" => crate::backend::subagent_watcher::events::SubagentAbandonedEvent,
    SubagentBlockPruned: "subagent:block_pruned" => crate::backend::subagent_watcher::events::SubagentBlockPrunedEvent,
    /// A resumed pane's subagent backfill started or finished.
    SubagentBackfillStatus: super::mps::EVENT_SUBAGENT_BACKFILL_STATUS => crate::backend::subagent_watcher::events::SubagentBackfillStatusEvent,
    /// A block file changed: terminal output, an agent transcript append (`blockcontroller`).
    BlockFile: super::mps::EVENT_BLOCK_FILE => super::mps::WSFileEventData,
    /// One pane's process-tree stats sample (`sysinfo`).
    BlockStats: super::mps::EVENT_BLOCK_STATS => crate::backend::rpc_types::TimeSeriesData,
    /// A wave object changed (`wcore::event`).
    MuxObjUpdated: super::mps::EVENT_MUX_OBJ_UPDATE => crate::backend::obj::MuxObjUpdate,
    /// A file an editor pane shows changed on disk.
    EditorFileChanged: crate::backend::editor_file_watcher::EVENT_EDITOR_FILE_CHANGED => PathChangedEvent,
    /// A file a media pane shows changed on disk.
    MediaFileChanged: crate::backend::media_file_watcher::EVENT_MEDIA_FILE_CHANGED => PathChangedEvent,
    /// A folder a Files pane lists changed.
    FilesChanged: crate::backend::files_watcher::EVENT_FILES_CHANGED => FilesChangedEvent,
    /// A copy/move/delete job's progress (`fs_ops::jobs`).
    FilesOp: crate::backend::fs_ops::jobs::EVENT_FILES_OP => crate::backend::rpc_types::FsOpEvent,
    /// One line of an install's output, or its end (`install.start`, `system.install`).
    InstallChunk: "install_chunk" => crate::server::install_handlers::InstallChunkEvent,
    /// A Shell command started (`server::http_shell`).
    ShellNodeCreate: super::mps::EVENT_SHELL_NODE_CREATE => crate::backend::shell_node::ShellNodeCreateEvent,
    /// One line a Shell command wrote, or its exit (`backend::shell_node`).
    ShellChunk: super::mps::EVENT_SHELL_CHUNK => crate::backend::shell_node::ShellChunkEvent,
    /// An attachment batch's progress, per item (`backend::attachments`).
    AttachmentProgress: super::mps::EVENT_ATTACHMENT_PROGRESS => crate::backend::rpc_types::AttachmentProgressEvent,
    AttachmentReady: super::mps::EVENT_ATTACHMENT_READY => crate::backend::rpc_types::AttachmentReadyEvent,
    AttachmentFailed: super::mps::EVENT_ATTACHMENT_FAILED => crate::backend::rpc_types::AttachmentFailedEvent,
    AttachmentBatchDone: super::mps::EVENT_ATTACHMENT_BATCH_DONE => crate::backend::rpc_types::AttachmentBatchDoneEvent,
    /// A connection's status changed (`backend::remote::status`).
    ConnChange: super::mps::EVENT_CONN_CHANGE => crate::backend::rpc_types::ConnStatus,

    // Signals with no payload: something changed, and a subscriber refetches what it shows.
    /// The agents list changed.
    AgentsChanged: "agents:changed" => (),
    /// An agent's stored content changed (`setagentcontent`).
    AgentContentChanged: "agentcontent:changed" => (),
    /// An entry was appended to an agent's history.
    AgentHistoryChanged: "agenthistory:changed" => (),
    /// A named agent was hidden.
    NamedAgentsChanged: "namedagents:changed" => (),
    /// The skills installed for an agent changed.
    AgentSkillsChanged: "agentskills:changed" => (),
    /// Memory entries changed.
    MemoriesChanged: "memories:changed" => (),
    /// The skill library changed.
    SkillsChanged: "skills:changed" => (),
    /// The MCP servers changed.
    McpChanged: "mcp:changed" => (),
    /// The provider accounts changed.
    IdentityAccountsChanged: "identityaccounts:changed" => (),
    /// Scheduled jobs changed.
    CronChanged: super::mps::EVENT_CRON_CHANGED => (),
    /// The drones changed.
    DronesChanged: "drones:changed" => (),
    /// The configured remotes changed.
    RemotesChanged: super::mps::EVENT_REMOTES_CHANGE => (),
    /// The process broker's set of tracked blocks changed.
    TrackedBlocksChanged: crate::broker::process::EVENT_TRACKED_BLOCKS_CHANGED => (),
    /// The work queue changed.
    WorkQueueChanged: "workqueue:changed" => (),
}

/// An event's `data`: its payload as JSON, or none for an event that carries none (`()`, a
/// pure "something changed" signal), so such an event goes out with no `data` field at all.
pub fn payload_data<P: Serialize>(payload: &P) -> Option<serde_json::Value> {
    serde_json::to_value(payload).ok().filter(|v| !v.is_null())
}

/// `editor:file_changed` / `media:file_changed`: the file that changed.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PathChangedEvent {
    pub path: String,
}

/// `files:changed`: the folder that changed, as the Files pane shows it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FilesChangedEvent {
    pub dir: String,
}

/// The `agent-resume-retry` event's payload (`mps_events::AgentResumeRetry`).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ResumeRetryEvent {
    #[ts(type = r#""retrying" | "resolved""#)]
    pub status: String,
    /// When the retry began; only on `retrying`.
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    #[ts(rename = "startedAt", optional)]
    pub started_at: Option<String>,
}

/// `schema()` as the JSON the event-schema export writes: one row per event, sorted by name.
#[cfg(test)]
pub fn schema_json() -> serde_json::Value {
    let mut rows = schema();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    serde_json::Value::Array(
        rows.into_iter()
            .map(|(name, ts)| serde_json::json!({ "event": name, "payloadTs": { "name": ts.name, "imports": ts.imports } }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_is_listed_once() {
        let names: Vec<&str> = schema().into_iter().map(|(n, _)| n).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "an event listed twice: {names:?}");
    }

    #[test]
    fn payloads_name_their_generated_types() {
        let rows = schema();
        let sys = rows.iter().find(|(n, _)| *n == "sysinfo").expect("sysinfo");
        assert_eq!(sys.1.name, "TimeSeriesData");
        assert_eq!(sys.1.imports, vec!["TimeSeriesData".to_string()]);
    }
}
