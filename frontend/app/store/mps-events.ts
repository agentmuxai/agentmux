// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// MPS event names: every `EVENT_*` constant in crates/srv/src/backend/mps.rs,
// plus the events other backend modules publish by name (`files:changed`,
// `waveobj:batchedupdates`, …). mps-events.test.ts checks them against the
// backend source and fails a subscription that spells a name by hand.

export const WpsEvent = {
    BlockFile: "blockfile",
    BlockClose: "blockclose",
    ConnChange: "connchange",
    /** The Remotes pane's list changed (SPEC_REMOTES_PANE_2026_10_05.md §4.6). */
    RemotesChange: "remoteschange",
    SysInfo: "sysinfo",
    ControllerStatus: "controllerstatus",
    // What an agent process was spawned with (model, effort, permission) and
    // whether a restart is pending. Mirrors EVENT_AGENT_RUNTIME in mps.rs.
    AgentRuntime: "agentruntime",
    // Where an agent process auto-compacts, as its CLI reports it. Mirrors
    // EVENT_AGENT_CONTEXT_USAGE in mps.rs.
    AgentContextUsage: "agentcontextusage",
    // The agent's turn as the user sees it, over its CLI passes. Mirrors
    // EVENT_AGENT_TURN in mps.rs; payload parsed by agent-pane-state/turn-ledger.ts.
    AgentTurn: "agentturn",
    MuxObjUpdate: "waveobj:update",
    // One WS frame carrying an ARRAY of MuxObjUpdates from a single atomic
    // backend transition (e.g. CloseTab's [update workspace, delete tab]
    // pair) — applied in one Solid batch() flush so the UI can't paint a
    // half-applied intermediate state. Mirrors
    // WS_EVENT_MUX_OBJ_BATCHED_UPDATES in
    // crates/srv/src/backend/eventbus.rs. See
    // docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7.
    MuxObjBatchedUpdates: "waveobj:batchedupdates",
    InstallProgress: "install_progress",
    Config: "config",
    // Sent once per WebSocket connect, after config: the srv reached and its
    // machine (crates/srv/src/srv_info.rs). Handled as it arrives, before any
    // subscription exists (app/store/srv-info.ts noteSrvInfoMessage).
    SrvInfo: "srvinfo",
    UserInput: "userinput",
    AgentMessageAccepted: "agent-message-accepted",
    // Image attachment processing, scoped `block:<id>` of the pane that
    // started the batch. Payloads in frontend/types/rpc/Attachment*Event.ts;
    // see docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.5.
    AttachmentProgress: "attachment:progress",
    AttachmentReady: "attachment:ready",
    AttachmentFailed: "attachment:failed",
    AttachmentBatchDone: "attachment:batch-done",
    RouteGone: "route:gone",
    BlockStats: "blockstats",
    AgentFailure: "agentfailure",
    ShellNodeCreate: "shell_node_create",
    ShellChunk: "shell_chunk",
    // Published by the `PreCompact` hook (`agentmux-bashwrap precompact`) the
    // instant Claude Code begins compacting — see
    // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2.
    CompactionStarted: "compaction_started",
    // Published by the persistent controller's stale-`--resume` recovery
    // path — `{status:"retrying"|"resolved"}`. See
    // docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
    AgentResumeRetry: "agent-resume-retry",
    // Published by SubagentWatcher::scan_session_subagents (pane-reopen
    // cold-backfill entry point) — `{status:"started"|"done"}`. See
    // docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md §5.
    SubagentBackfillStatus: "subagent:backfill_status",
    BlockActivity: "block:activity",
    // Fired when a file open in at least one editor/preview tab changes on
    // disk. Payload: `{ path }` — a wake signal only, no content; handlers
    // re-fetch via ReadEditorFileCommand. See
    // docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    EditorFileChanged: "editor:file_changed",
    // Fired when a file matching a Media pane's extension filter is
    // created/modified in a directory it's watching. Payload: `{ path }` —
    // a wake signal only. See docs/specs/SPEC_MEDIA_PANE_2026_07_26.md.
    MediaFileChanged: "media:file_changed",
    // Fired when a folder a Files pane watches (`FsWatchCommand`) changes:
    // an entry created, modified, removed or renamed. Coalesced per folder;
    // also fired for every watched folder after a watcher overflow, so it
    // means "re-list", never a diff. Payload: `{ dir }`, the folder's path as
    // `FsListCommand` returns it. Scoped `block:<id>`. See
    // docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3.
    FilesChanged: "files:changed",
    // Progress of a copy/move job started with `FsOpStartCommand`. Payload:
    // `FsOpEvent` (`{ op_id, kind, state, done_items, total_items,
    // done_bytes, total_bytes, current?, conflict?, error?, failures? }`).
    // `running` is throttled to one per 100 ms; `done`, `failed` and
    // `canceled` are final. Scoped `block:<id>`. See
    // docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §7.1.
    FilesOp: "files:op",
    UpgradeMigrationEvent:     "upgrade:migration-event",
    UpgradeMigrationsComplete: "upgrade:migrations-complete",
    UpgradeMigrationsFailed:   "upgrade:migrations-failed",
    UpgradeSagaVacuumDone:     "upgrade:saga-vacuum-done",
    // Published by `handle_muxspect_dock_clear` in response to a
    // `muxspect dock clear` request. Scoped `block:<id>` — only a
    // renderer currently displaying that block receives it. Payload:
    // `{ node_id }`. See
    // docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.2.
    DockClear: "dock:clear",
    // Published whenever a block's db_background_tasks state changes
    // (observed, pid recorded, or completed) — an invalidation signal
    // only, no task data (see `publish_background_task_updated` in
    // websocket.rs). Handlers re-fetch via ListBackgroundTasksCommand.
    // See docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.2.
    BackgroundTaskUpdated: "background-task-updated",
    // A short, model-voiced line describing something AgentMux did on its own
    // (first consumer: a tool call the harness detached to the background).
    // Unlike BackgroundTaskUpdated above this carries its payload —
    // `{ block_id, kind, text }` — because there is no list query to re-read,
    // so an invalidation ping would have nothing to invalidate.
    AmbientNarration: "ambient-narration",
    // Streamed answer chunks for a `/btw <question>` side question — scoped
    // `block:<blockId>:btw:<requestId>` (NOT plain `block:<blockId>`, unlike
    // most other block-scoped events here: a pane can have more than one
    // `/btw` in flight, and each overlay must only see its own answer).
    // Mirrors `mps::EVENT_BTW_ANSWER_CHUNK`
    // (crates/srv/src/server/agent_handlers/side_question.rs's
    // `publish_chunk`). Payload:
    // `{ blockId: string, requestId: string, event: AgentEvent, done: boolean }`
    // — `event` is the SAME tagged `AgentEvent` union every normal turn
    // streams (`type: "assistant_text"` with `delta`, `"done"` with the
    // final `response`, `"error"` with `message`, etc. — see
    // `frontend/types/srv-types.d.ts`'s `AgentEvent`). The outer `done: true`
    // (not `event.type === "done"`) is the authoritative completion signal
    // `components/BtwOverlay.tsx` waits on — it also fires on a terminal
    // `error` event, which `event.type` alone would not indicate as "done".
    BtwAnswerChunk: "btw_answer_chunk",
    // Published outside mps.rs's EVENT_* constants (by the module that owns
    // each), and subscribed to by name; listed so no subscription spells one
    // by hand (mps-events.test.ts).
    AgentProcessAdded: "agent:process-added",
    AgentProcessExited: "agent:process-exited",
    AgentProgress: "agent:progress",
    AgentReactiveRegistered: "agent:reactive-registered",
    AgentReactiveUnregistered: "agent:reactive-unregistered",
    AgentsChanged: "agents:changed",
    CronChanged: "cron_changed",
    DispatchActivity: "dispatch:activity",
    DispatchUpdated: "dispatch:updated",
    IdentityAccountsChanged: "identityaccounts:changed",
    InstallChunk: "install_chunk",
    /** Widget packages changed (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8). */
    WidgetPackages: "widgetpackages",
    /** A widget package's storage changed: `{ id, keys }`. */
    WidgetStorage: "widgetstorage",
    /** Agents' widget install requests waiting for the user: `{ requests }`. */
    WidgetRequests: "widgetrequests",
    /** A call to action srv holds open started or ended (the waiting tone). */
    UserAttention: "userattention",
    LanInstances: "laninstances",
    LanInstancesError: "laninstances:error",
    LanInstancesFirewall: "laninstances:firewall",
    LanInstancesHealth: "laninstances:health",
    LspMessage: "lsp:message",
    McpChanged: "mcp:changed",
    MemoriesChanged: "memories:changed",
    OpenFloatingPane: "openfloatingpane",
    ProcessBrokerTrackedBlocksChanged: "processbroker:tracked-blocks-changed",
    SkillsChanged: "skills:changed",
    SubagentAbandoned: "subagent:abandoned",
    SubagentBlockPruned: "subagent:block_pruned",
    SubagentCompleted: "subagent:completed",
    SubagentNamed: "subagent:named",
    SubagentSpawned: "subagent:spawned",
    SubagentUpdated: "subagent:updated",
    ToolChunk: "tool_chunk",
    ViewerPaired: "viewer:paired",
    AgentShutdown: "agent:shutdown",
    AgentShutdownPending: "agent:shutdown-pending",
    AgentShutdownPendingCleared: "agent:shutdown-pending-cleared",
    AmbientSpent: "ambient:spent",
    BlockReveal: "block:reveal",
    MuxbusStatus: "muxbus:status",
    NotificationActivate: "notification:activate",
    NotificationState: "notification:state",
    // Published by the frontend itself (singleton-modal.ts).
    SingletonClaim: "singleton:claim",
} as const;
