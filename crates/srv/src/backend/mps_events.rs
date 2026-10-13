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
//!
//! An event sent under a name per id (`agent:memory:changed:<agent id>`) is a [`PerIdEvent`],
//! declared in the `per_id_events!` list by the name before the id, and published with
//! [`Broker::publish_event_for`](super::mps::Broker::publish_event_for).

use serde::Serialize;

#[cfg(test)]
use crate::backend::rpc::schema::{ts_type, TsType};

/// A broker event with a typed payload.
pub trait Event {
    /// The wire name (`MuxEvent::event`).
    const NAME: &'static str;
    type Payload: Serialize + ts_rs::TS + 'static;
}

/// A broker event sent under a name per id: its prefix, then the id.
pub trait PerIdEvent {
    /// The wire name before the id (`agent:memory:changed:`).
    const PREFIX: &'static str;
    type Payload: Serialize + ts_rs::TS + 'static;
}

macro_rules! per_id_events {
    ($($(#[$doc:meta])* $marker:ident: $prefix:expr => $payload:ty,)*) => {
        $(
            $(#[$doc])*
            pub struct $marker;
            impl PerIdEvent for $marker {
                const PREFIX: &'static str = $prefix;
                type Payload = $payload;
            }
        )*
        /// Every per-id event: its prefix and its payload as TypeScript.
        #[cfg(test)]
        pub fn per_id_schema() -> Vec<(&'static str, TsType)> {
            vec![$(($prefix, ts_type::<$payload>())),*]
        }
    };
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
    /// A pane's background tasks changed; the dashboard reads the list again.
    BackgroundTaskUpdated: "background-task-updated" => BlockRefEvent,
    /// A line of narration for an agent pane (`continued_note`, the narrate route).
    AmbientNarration: "ambient-narration" => AmbientNarrationEvent,
    /// A pane's reported MCP servers changed (`mcp_observed`).
    McpObserved: "mcp:observed" => BlockRefEvent,
    /// What the notification tray shows (`notify::router`).
    NotificationState: crate::backend::notify::router::EVENT_NOTIFICATION_STATE => crate::backend::notify::policy::TrayState,
    /// An agent's checklist and current tool (`reactive::progress_watcher`).
    AgentProgress: crate::backend::reactive::progress_watcher::EVENT_AGENT_PROGRESS => AgentProgressEvent,
    /// The installed widget packages.
    WidgetPackages: super::mps::EVENT_WIDGET_PACKAGES => WidgetPackagesEvent,
    /// A widget package's storage changed: which keys, never their values.
    WidgetStorage: super::mps::EVENT_WIDGET_STORAGE => WidgetStorageEvent,
    /// The agents' widget install requests waiting for the user.
    WidgetRequests: super::mps::EVENT_WIDGET_REQUESTS => WidgetRequestsEvent,
    /// A pane's process status changed (`broker::process`).
    ProcessStatusChanged: crate::broker::process::EVENT_STATUS_CHANGED => crate::broker::process::ProcessStatus,
    /// An agent instance's identity has no direct links (`identity::resolver::inject`); a diagnostic.
    IdentityNoDirectLinks: "identity:no-direct-links" => NoDirectLinksEvent,
    /// One streamed piece of a side question's answer (`agent_handlers::side_question`).
    BtwAnswerChunk: super::mps::EVENT_BTW_ANSWER_CHUNK => BtwAnswerChunkEvent,
    /// `muxspect dock clear` removed a node from a pane's dock.
    DockClear: super::mps::EVENT_DOCK_CLEAR => DockClearEvent,
    /// A reactive agent pane registered, or went (the Swarm refreshes).
    AgentReactiveRegistered: "agent:reactive-registered" => BlockRefEvent,
    AgentReactiveUnregistered: "agent:reactive-unregistered" => BlockRefEvent,
    /// One protocol frame from an app-server agent (`app_server_controller`).
    AppServerOutput: "output" => ProtocolFrame,
    /// srv's version and machine, sent once on every WebSocket connect.
    SrvInfo: "srvinfo" => crate::srv_info::SrvInfo,
    /// The full config, redacted for the renderer: on connect, and when it changes.
    Config: super::mps::EVENT_CONFIG => ConfigEvent,
    /// What one ambient model call spent (`ambient::spend`).
    AmbientSpent: crate::ambient::spend::EVENT_AMBIENT_SPENT => crate::ambient::spend::AmbientSpentEvent,
    /// A process joined an agent pane's tree, or left it (`process_tracker::registry`).
    AgentProcessAdded: "agent:process-added" => ProcessAddedEvent,
    AgentProcessExited: "agent:process-exited" => ProcessExitedEvent,
    /// A shutdown waiting for the user to keep the agent, and its end (`sagas::pending_shutdown`).
    AgentShutdownPending: crate::sagas::pending_shutdown::EVENT_SHUTDOWN_PENDING => crate::sagas::pending_shutdown::PendingView,
    AgentShutdownPendingCleared: crate::sagas::pending_shutdown::EVENT_SHUTDOWN_PENDING_CLEARED => crate::sagas::pending_shutdown::PendingClearedEvent,
    /// An OS notification to show, retract or act on (`notify::router`).
    Notification: crate::backend::notify::router::EVENT_NOTIFICATION => crate::backend::notify::policy::Notification,
    NotificationRetract: crate::backend::notify::router::EVENT_NOTIFICATION_RETRACT => crate::backend::notify::router::NotificationRetractEvent,
    NotificationActivate: crate::backend::notify::router::EVENT_NOTIFICATION_ACTIVATE => crate::backend::notify::router::NotificationActivateEvent,
    /// Ask one window to open a pane as its own floating OS window (`app_api::pane`).
    OpenFloatingPane: "openfloatingpane" => OpenFloatingPaneEvent,
    /// The LAN peers this instance sees (`lan_discovery`).
    LanInstances: "laninstances" => crate::backend::lan_discovery::LanInstanceList,
    /// LAN discovery couldn't start, or a daemon rebuild failed.
    LanInstancesError: "laninstances:error" => crate::backend::lan_discovery::LanDiscoveryError,
    /// Whether this instance is discoverable on every LAN interface (the mDNS watchdog).
    LanInstancesHealth: "laninstances:health" => crate::backend::lan_discovery::LanDiscoverability,
    /// The OS firewall's verdict on LAN peers reaching this machine (`lan_firewall`).
    LanInstancesFirewall: "laninstances:firewall" => crate::backend::lan_firewall::LanFirewall,
    /// Several object updates from one atomic transition, applied in one flush (`EventBus::broadcast_mux_obj_updates`).
    MuxObjBatchedUpdates: super::eventbus::WS_EVENT_MUX_OBJ_BATCHED_UPDATES => crate::backend::obj::MuxObjBatch,
    /// A line of an agent CLI's install progress, for its pane (`mps::publish_install_progress`).
    InstallProgress: super::mps::EVENT_INSTALL_PROGRESS => InstallProgressEvent,
    /// An agent pane's activity, from its terminal title (`mps::publish_block_activity`).
    BlockActivity: super::mps::EVENT_BLOCK_ACTIVITY => BlockActivityEvent,
    /// One message from a language server (`lsp::supervisor`).
    LspMessage: "lsp:message" => LspMessageEvent,
    /// Cloud delivery's state changed (`muxbus::delivery_status`).
    MuxbusStatus: crate::muxbus::delivery_status::EVENT_MUXBUS_STATUS => crate::backend::rpc_types::MuxBusDeliveryStatus,
    /// A mobile viewer paired with this machine (`http_viewer`).
    ViewerPaired: "viewer:paired" => ViewerPairedEvent,
    /// Tell a window's renderer to reveal a block (`notify::router`).
    BlockReveal: crate::backend::notify::router::EVENT_BLOCK_REVEAL => crate::backend::notify::router::BlockRevealEvent,

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

per_id_events! {
    /// An agent's native memory files changed (`agent:memory:changed:<agent id>`).
    AgentMemoryChanged: "agent:memory:changed:" => (),
    /// An agent's identity links changed (`agentidentities:changed:<agent id>`).
    AgentIdentitiesChanged: "agentidentities:changed:" => (),
    /// A definition's instances changed (`agentinstances:changed:<definition id>`).
    AgentInstancesChanged: "agentinstances:changed:" => (),
    /// An agent's provider login was unlinked (`agentcredentials:revoked:<agent id>`).
    AgentCredentialsRevoked: "agentcredentials:revoked:" => CredentialsRevokedEvent,
    /// One step of a drone run (`dronerun:<run id>`).
    DroneRun: "dronerun:" => crate::drone::executor::engine::RunEvent,
    /// An agent pane's output, translated (`agent_event:<block id>`).
    AgentEventStream: "agent_event:" => crate::agents::types::AgentEvent,
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

/// An event about one pane: `background-task-updated`, `mcp:observed`, `agent:reactive-(un)registered`.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockRefEvent {
    pub block_id: String,
}

/// `ambient-narration`: a line to show in an agent pane.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AmbientNarrationEvent {
    pub block_id: String,
    pub kind: String,
    pub text: String,
}

/// `agent:progress`: an agent's checklist and the tool it is running.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AgentProgressEvent {
    pub agent_id: String,
    pub block_id: String,
    pub todos: Vec<crate::backend::reactive::progress_watcher::TodoItem>,
    /// How many rows the cap dropped.
    pub todos_truncated: usize,
    pub todos_partial: bool,
    pub current_tool: Option<String>,
    #[ts(type = "number")]
    pub ts: u64,
}

/// `widgetpackages`: every installed widget package.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetPackagesEvent {
    pub packages: Vec<crate::backend::widget_packages::WidgetPackageInfo>,
}

/// `widgetstorage`: the keys of a widget package's storage that changed.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetStorageEvent {
    pub id: String,
    pub keys: Vec<String>,
}

/// `widgetrequests`: the install requests waiting for the user.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetRequestsEvent {
    pub requests: Vec<crate::backend::widget_requests::WidgetInstallRequest>,
}

/// `identity:no-direct-links`: the instance and the identity that has no direct links.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NoDirectLinksEvent {
    pub definition_id: String,
    pub identity_id: String,
}

/// `btw_answer_chunk`: one event of a side question's answer. `done` (not `event.type`) says
/// the answer is complete.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct BtwAnswerChunkEvent {
    pub block_id: String,
    pub request_id: String,
    pub event: crate::agents::types::AgentEvent,
    pub done: bool,
}

/// `dock:clear`: the dock node to remove.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DockClearEvent {
    pub node_id: String,
}

/// `config`: the full config, redacted for the renderer.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ConfigEvent {
    #[ts(type = "FullConfigType")]
    pub fullconfig: serde_json::Value,
}

/// `install_progress`: one line of install progress.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InstallProgressEvent {
    pub message: String,
}

/// `block:activity`: what an agent pane is doing, from its terminal title.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct BlockActivityEvent {
    pub block_id: String,
    pub activity: String,
}

/// `lsp:message`: one JSON-RPC message from a language server, as it sent it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LspMessageEvent {
    pub server_id: String,
    #[ts(type = "unknown")]
    pub message: serde_json::Value,
}

/// `viewer:paired`: the device that just paired.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ViewerPairedEvent {
    pub device_id: String,
    pub device_name: String,
}

/// `openfloatingpane`: the pane to float, now alone in its own workspace.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct OpenFloatingPaneEvent {
    pub block_id: String,
    pub workspace_id: String,
}

/// `agent:process-added`: a process joined an agent pane's tree.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ProcessAddedEvent {
    pub block_id: String,
    pub process: crate::backend::process_tracker::TrackedProcess,
}

/// `agent:process-exited`: a process left an agent pane's tree.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ProcessExitedEvent {
    pub block_id: String,
    pub pid: u32,
}

/// `output`: one app-server protocol frame, as the agent sent it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(transparent)]
pub struct ProtocolFrame(#[ts(type = "unknown")] pub serde_json::Value);

/// `agentcredentials:revoked:<agent id>`: the agent still holds the provider's tokens until it restarts.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct CredentialsRevokedEvent {
    /// Always `true`.
    #[ts(type = "true")]
    pub credentials_revoked: bool,
    pub provider: String,
    /// The account unlinked, when one account was.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account_id: Option<String>,
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

/// `schema()` and `per_id_schema()` as the JSON the event-schema export writes: one row per event,
/// sorted by name. A per-id event's row names its prefix, with `perId: true`.
#[cfg(test)]
pub fn schema_json() -> serde_json::Value {
    let mut rows: Vec<(&str, TsType, bool)> = schema().into_iter().map(|(n, t)| (n, t, false)).collect();
    rows.extend(per_id_schema().into_iter().map(|(n, t)| (n, t, true)));
    rows.sort_by(|a, b| a.0.cmp(b.0));
    serde_json::Value::Array(
        rows.into_iter()
            .map(|(name, ts, per_id)| {
                let mut row = serde_json::json!({ "event": name, "payloadTs": { "name": ts.name, "imports": ts.imports } });
                if per_id {
                    row["perId"] = true.into();
                }
                row
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_is_listed_once() {
        let names: Vec<&str> = schema().into_iter().chain(per_id_schema()).map(|(n, _)| n).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "an event listed twice: {names:?}");
    }

    #[test]
    fn payloads_that_replaced_json_serialize_the_same() {
        let progress = AgentProgressEvent {
            agent_id: "a1".into(),
            block_id: "b1".into(),
            todos: vec![],
            todos_truncated: 2,
            todos_partial: false,
            current_tool: None,
            ts: 5,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            serde_json::json!({ "agentId": "a1", "blockId": "b1", "todos": [], "todosTruncated": 2, "todosPartial": false, "currentTool": null, "ts": 5 })
        );
        let chunk = BtwAnswerChunkEvent {
            block_id: "b1".into(),
            request_id: "r1".into(),
            event: crate::agents::types::AgentEvent::Error { message: "no".into() },
            done: true,
        };
        assert_eq!(
            serde_json::to_value(&chunk).unwrap(),
            serde_json::json!({ "blockId": "b1", "requestId": "r1", "event": { "type": "error", "message": "no" }, "done": true })
        );
        let frame = ProtocolFrame(serde_json::json!({ "method": "turn/started" }));
        assert_eq!(serde_json::to_value(&frame).unwrap(), serde_json::json!({ "method": "turn/started" }));
    }

    #[test]
    fn lan_payloads_serialize_as_the_json_they_replace() {
        use crate::backend::eventbus::EventBus;
        use crate::backend::lan_discovery::{LanDiscoverability, LanDiscoveryError, LanInstanceList};
        let empty = EventBus::typed_ws_event::<LanInstances>(&LanInstanceList(Vec::new()), String::new());
        assert_eq!((empty.eventtype.as_str(), empty.data), ("laninstances", Some(serde_json::json!([]))));
        let error = LanDiscoveryError { error: "bind failed".into() };
        assert_eq!(serde_json::to_value(&error).unwrap(), serde_json::json!({ "error": "bind failed" }));
        let health = LanDiscoverability { state: "degraded".into(), missing: vec!["192.0.2.7".into()], rebuilds: 1 };
        assert_eq!(serde_json::to_value(&health).unwrap(), serde_json::json!({ "state": "degraded", "missing": ["192.0.2.7"], "rebuilds": 1 }));
    }

    #[test]
    fn raw_event_payloads_serialize_as_the_json_they_replace() {
        use crate::backend::eventbus::EventBus;
        let progress = EventBus::typed_ws_event::<InstallProgress>(&InstallProgressEvent { message: "npm i".into() }, "block:b1");
        assert_eq!((progress.eventtype.as_str(), progress.oref.as_str()), ("install_progress", "block:b1"));
        assert_eq!(progress.data, Some(serde_json::json!({ "message": "npm i" })));
        let activity = BlockActivityEvent { block_id: "b1".into(), activity: "auth refactor".into() };
        assert_eq!(serde_json::to_value(&activity).unwrap(), serde_json::json!({ "blockId": "b1", "activity": "auth refactor" }));
        let lsp = LspMessageEvent { server_id: "rust".into(), message: serde_json::json!({ "id": 1 }) };
        assert_eq!(serde_json::to_value(&lsp).unwrap(), serde_json::json!({ "server_id": "rust", "message": { "id": 1 } }));
        let paired = ViewerPairedEvent { device_id: "d1".into(), device_name: "Phone".into() };
        assert_eq!(serde_json::to_value(&paired).unwrap(), serde_json::json!({ "device_id": "d1", "device_name": "Phone" }));
        // The batch goes out as the bare array, as it did before.
        let update = crate::backend::obj::MuxObjUpdate { updatetype: "delete".into(), otype: "tab".into(), oid: "t1".into(), obj: None };
        let batch = EventBus::typed_ws_event::<MuxObjBatchedUpdates>(&crate::backend::obj::MuxObjBatch(vec![update]), String::new());
        assert_eq!(batch.eventtype, "waveobj:batchedupdates");
        assert_eq!(batch.data, Some(serde_json::json!([{ "updatetype": "delete", "otype": "tab", "oid": "t1" }])));
    }

    #[test]
    fn a_typed_frame_is_the_eventrecv_frame_srv_wrote_by_hand() {
        let frame = crate::backend::eventbus::EventBus::typed_frame::<Config>(&ConfigEvent { fullconfig: serde_json::json!({ "settings": {} }) });
        assert_eq!(
            frame,
            serde_json::json!({ "eventtype": "rpc", "data": { "command": "eventrecv", "data": { "event": "config", "data": { "fullconfig": { "settings": {} } } } } })
        );
        let activate = crate::backend::notify::router::NotificationActivateEvent {
            id: "n1".into(),
            block_id: "b1".into(),
            tab_id: "t1".into(),
            window_id: "w1".into(),
            workspace_id: None,
            at_ms: 7,
        };
        assert_eq!(
            serde_json::to_value(&activate).unwrap(),
            serde_json::json!({ "id": "n1", "block_id": "b1", "tab_id": "t1", "window_id": "w1", "workspace_id": null, "at_ms": 7 })
        );
    }

    #[test]
    fn payloads_name_their_generated_types() {
        let rows = schema();
        let sys = rows.iter().find(|(n, _)| *n == "sysinfo").expect("sysinfo");
        assert_eq!(sys.1.name, "TimeSeriesData");
        assert_eq!(sys.1.imports, vec!["TimeSeriesData".to_string()]);
    }
}
