// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// WPS event-name constants — mirrors agentmux-srv/src/backend/wps.rs:22-54.
// Use these instead of bare string literals so typos are caught at build time
// and grepping for an event name finds all its usages in one search.

export const WpsEvent = {
    BlockFile: "blockfile",
    BlockClose: "blockclose",
    ConnChange: "connchange",
    SysInfo: "sysinfo",
    ControllerStatus: "controllerstatus",
    WaveObjUpdate: "waveobj:update",
    // One WS frame carrying an ARRAY of WaveObjUpdates from a single atomic
    // backend transition (e.g. CloseTab's [update workspace, delete tab]
    // pair) — applied in one Solid batch() flush so the UI can't paint a
    // half-applied intermediate state. Mirrors
    // WS_EVENT_WAVE_OBJ_BATCHED_UPDATES in
    // agentmux-srv/src/backend/eventbus.rs. See
    // docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7.
    WaveObjBatchedUpdates: "waveobj:batchedupdates",
    InstallProgress: "install_progress",
    Config: "config",
    UserInput: "userinput",
    AgentMessageAccepted: "agent-message-accepted",
    RouteGone: "route:gone",
    BlockStats: "blockstats",
    AgentHealth: "agenthealth",
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
} as const;
