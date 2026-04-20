// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentControlBar - Collapsible runtime controls for permission mode,
 * model, and effort level. Changes take effect on the next turn.
 *
 * Also shows Archive / Export / Restore session management buttons
 * when the session has data (session:line_count > 0).
 */

import { createSignal, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import type { AgentRuntimeConfig, PermissionMode, ModelChoice, EffortLevel } from "../types";
import { DEFAULT_RUNTIME_CONFIG } from "../types";
import { getRuntimeConfig } from "../buildRuntimeArgs";

interface AgentControlBarProps {
    blockId: string;
    blockAtom: () => Block | undefined;
    providerId: string;
}

const PERMISSION_LABELS: Record<PermissionMode, string> = {
    bypass: "Bypass",
    auto: "Auto",
    acceptEdits: "Accept Edits",
    plan: "Plan",
    default: "Default",
};

const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color, #ef4444)",
    auto: "var(--accent-color, #3b82f6)",
    acceptEdits: "var(--warning-color, #eab308)",
    plan: "var(--success-color, #22c55e)",
    default: "var(--main-text-color)",
};

const MODEL_LABELS: Record<string, string> = {
    "": "Default",
    opus: "Opus",
    sonnet: "Sonnet",
    haiku: "Haiku",
};

const EFFORT_LABELS: Record<string, string> = {
    "": "Default",
    low: "Low",
    medium: "Medium",
    high: "High",
    max: "Max",
};

export const AgentControlBar = ({ blockId, blockAtom, providerId }: AgentControlBarProps): JSX.Element => {
    const [expanded, setExpanded] = createSignal(false);
    const [archiveBusy, setArchiveBusy] = createSignal(false);
    const [exportBusy, setExportBusy] = createSignal(false);
    const [restoreBusy, setRestoreBusy] = createSignal(false);

    const runtime = (): AgentRuntimeConfig => getRuntimeConfig(blockAtom()?.meta);

    const isNonDefault = (): boolean => {
        const r = runtime();
        return (
            r.permissionMode !== DEFAULT_RUNTIME_CONFIG.permissionMode ||
            r.model !== DEFAULT_RUNTIME_CONFIG.model ||
            r.effort !== DEFAULT_RUNTIME_CONFIG.effort
        );
    };

    const updateRuntime = async (patch: Partial<AgentRuntimeConfig>) => {
        const current = runtime();
        const updated = { ...current, ...patch };
        const oref = WOS.makeORef("block", blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: { "agent:runtime": updated },
            });
        } catch {
            // Silently ignore — settings will retry on next change
        }
    };

    const compactSummary = (): string => {
        const r = runtime();
        return [
            PERMISSION_LABELS[r.permissionMode],
            MODEL_LABELS[r.model] || r.model,
            `Effort: ${EFFORT_LABELS[r.effort] || r.effort}`,
        ].join(" · ");
    };

    // ── Session management helpers ──────────────────────────────────────────────

    const lineCount = (): number =>
        (blockAtom()?.meta?.["session:line_count"] as number | undefined) ?? 0;

    const isArchived = (): boolean => {
        const archivedAt = blockAtom()?.meta?.["session:archived_at"] as number | undefined;
        return (archivedAt ?? 0) > 0;
    };

    const archivedAtLabel = (): string => {
        const ts = blockAtom()?.meta?.["session:archived_at"] as number | undefined;
        if (!ts) return "";
        return new Date(ts).toLocaleString();
    };

    const handleArchive = async () => {
        if (archiveBusy()) return;
        setArchiveBusy(true);
        try {
            await RpcApi.SessionArchiveCommand(TabRpcClient, { block_id: blockId });
        } catch (e) {
            console.error("session:archive failed:", e);
        } finally {
            setArchiveBusy(false);
        }
    };

    const handleRestore = async () => {
        if (restoreBusy()) return;
        setRestoreBusy(true);
        try {
            await RpcApi.SessionRestoreCommand(TabRpcClient, { block_id: blockId });
        } catch (e) {
            console.error("session:restore failed:", e);
        } finally {
            setRestoreBusy(false);
        }
    };

    const handleExport = async () => {
        if (exportBusy()) return;
        setExportBusy(true);
        try {
            const result = await RpcApi.SessionExportCommand(TabRpcClient, { block_id: blockId });
            // Decode base64 and trigger a browser download
            const raw = atob(result.content);
            const bytes = new Uint8Array(raw.length);
            for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
            const blob = new Blob([bytes], { type: "application/x-ndjson" });
            const url = URL.createObjectURL(blob);
            const a = document.createElement("a");
            const ts = Date.now();
            a.href = url;
            a.download = `session-${blockId.slice(0, 8)}-${ts}.jsonl`;
            a.click();
            URL.revokeObjectURL(url);
        } catch (e) {
            console.error("session:export failed:", e);
        } finally {
            setExportBusy(false);
        }
    };

    // Only show for Claude provider (Phase 1)
    if (providerId !== "claude") return null;

    // 4.1 Graceful degradation: warn when session grows unwieldy (500K lines).
    // Browser still handles this via content-visibility, but we surface the state
    // so the user can archive + start fresh if they want a cleaner session.
    const LARGE_SESSION_THRESHOLD = 500_000;
    const isLargeSession = (): boolean =>
        !isArchived() && lineCount() >= LARGE_SESSION_THRESHOLD;

    // 4.2 Multi-day continuity: server sets `session:was_interrupted` when it
    // finds a stale `session:active_pid` on startup (i.e. the previous server
    // process died with a subprocess running). The next user message will
    // automatically `--resume` the session, so the banner just lets the user
    // know what happened and offers a "Dismiss" to clear the flag.
    const wasInterrupted = (): boolean =>
        (blockAtom()?.meta?.["session:was_interrupted"] as boolean | undefined) === true;

    const dismissInterrupted = async () => {
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", blockId),
                meta: { "session:was_interrupted": null } as MetaType,
            });
        } catch (e) {
            console.error("failed to clear was_interrupted:", e);
        }
    };

    return (
        <div class="agent-control-bar">
            {/* ── Interrupted-session recovery banner (4.2 multi-day continuity) ── */}
            <Show when={wasInterrupted()}>
                <div class="agent-interrupted-banner">
                    <span class="agent-interrupted-label">
                        Session was interrupted by a restart. Your next message will resume it.
                    </span>
                    <button
                        class="agent-session-btn agent-session-btn-dismiss"
                        onClick={dismissInterrupted}
                        title="Dismiss this notice"
                    >
                        Dismiss
                    </button>
                </div>
            </Show>

            {/* ── Large session warning (4.1 graceful degradation) ── */}
            <Show when={isLargeSession()}>
                <div class="agent-large-session-banner">
                    <span class="agent-large-session-label">
                        Session has {lineCount().toLocaleString()} lines. Consider archiving to free disk space.
                    </span>
                    <button
                        class="agent-session-btn agent-session-btn-archive"
                        disabled={archiveBusy()}
                        onClick={handleArchive}
                        title="Archive this session and start fresh"
                    >
                        {archiveBusy() ? "Archiving…" : "Archive"}
                    </button>
                </div>
            </Show>

            {/* ── Archived badge (shown when session is archived) ── */}
            <Show when={isArchived()}>
                <div class="agent-archived-banner">
                    <span class="agent-archived-label" title={`Archived at ${archivedAtLabel()}`}>
                        Archived
                    </span>
                    <button
                        class="agent-session-btn agent-session-btn-restore"
                        disabled={restoreBusy()}
                        onClick={handleRestore}
                        title="Restore session data from archive"
                    >
                        {restoreBusy() ? "Restoring…" : "Restore"}
                    </button>
                    <button
                        class="agent-session-btn agent-session-btn-export"
                        disabled={exportBusy()}
                        onClick={handleExport}
                        title="Export session as .jsonl"
                    >
                        {exportBusy() ? "Exporting…" : "Export"}
                    </button>
                </div>
            </Show>

            <div
                class="agent-control-bar-header"
                onClick={() => setExpanded(!expanded())}
            >
                <span class="agent-control-chevron">{expanded() ? "▾" : "▸"}</span>
                <span class="agent-control-summary">
                    <Show when={!expanded()}>
                        {compactSummary()}
                        <Show when={isNonDefault()}>
                            <span class="agent-control-nondefault" title="Non-default settings active">*</span>
                        </Show>
                    </Show>
                    <Show when={expanded()}>
                        Controls
                    </Show>
                </span>
            </div>
            <Show when={expanded()}>
                <div class="agent-control-bar-body">
                    <div class="agent-control-row">
                        <label class="agent-control-label">Mode</label>
                        <select
                            class="agent-control-select"
                            value={runtime().permissionMode}
                            style={{ "border-left": `3px solid ${PERMISSION_COLORS[runtime().permissionMode]}` }}
                            onChange={(e) => updateRuntime({ permissionMode: e.target.value as PermissionMode })}
                        >
                            <option value="bypass">Bypass (no prompts)</option>
                            <option value="auto">Auto (AI classifier)</option>
                            <option value="acceptEdits">Accept Edits</option>
                            <option value="plan">Plan (read-only)</option>
                            <option value="default">Default (prompt all)</option>
                        </select>
                    </div>
                    <div class="agent-control-row">
                        <label class="agent-control-label">Model</label>
                        <select
                            class="agent-control-select"
                            value={runtime().model}
                            onChange={(e) => updateRuntime({ model: e.target.value as ModelChoice })}
                        >
                            <option value="opus">Opus</option>
                            <option value="sonnet">Sonnet</option>
                            <option value="haiku">Haiku</option>
                        </select>
                    </div>
                    <div class="agent-control-row">
                        <label class="agent-control-label">Effort</label>
                        <select
                            class="agent-control-select"
                            value={runtime().effort}
                            onChange={(e) => updateRuntime({ effort: e.target.value as EffortLevel })}
                        >
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                            <option value="max">Max (Opus only)</option>
                        </select>
                    </div>

                    {/* ── Session management buttons (shown when there is history) ── */}
                    <Show when={lineCount() > 0 && !isArchived()}>
                        <div class="agent-control-row agent-control-row-session">
                            <label class="agent-control-label">Session</label>
                            <div class="agent-session-actions">
                                <button
                                    class="agent-session-btn agent-session-btn-archive"
                                    disabled={archiveBusy()}
                                    onClick={handleArchive}
                                    title="Compress and archive session history to free disk space"
                                >
                                    {archiveBusy() ? "Archiving…" : "Archive"}
                                </button>
                                <button
                                    class="agent-session-btn agent-session-btn-export"
                                    disabled={exportBusy()}
                                    onClick={handleExport}
                                    title="Download session history as .jsonl"
                                >
                                    {exportBusy() ? "Exporting…" : "Export"}
                                </button>
                            </div>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
};
