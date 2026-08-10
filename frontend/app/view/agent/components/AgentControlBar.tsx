// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentControlBar — session-lifecycle controls for the agent pane, shown in
 * the composer details region (below the Log toggle).
 *
 * Runtime controls (permission Mode, Model, Effort) used to live here behind a
 * collapsible "Controls" chevron. They were promoted to top-level dropdowns in
 * `AgentComposerStrip` and this nested duplicate was removed — see
 * SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02. What remains is purely session
 * management: the interrupted / large-session / archived banners and the
 * Archive / Export / Restore actions. These render directly (no inner
 * chevron) — the Log button is the single toggle for this whole region.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";

interface AgentControlBarProps {
    blockId: string;
    blockAtom: () => Block | undefined;
    providerId: string;
    /** Open the pane's Agent History view (spec §4.2 entry point) —
     *  reachable even when the pane has no session boundary. */
    onOpenHistory?: () => void;
}

export const AgentControlBar = ({ blockId, blockAtom, providerId, onOpenHistory }: AgentControlBarProps): JSX.Element => {
    const [archiveBusy, setArchiveBusy] = createSignal(false);
    const [exportBusy, setExportBusy] = createSignal(false);
    const [restoreBusy, setRestoreBusy] = createSignal(false);

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

    // Only show session-management controls for Claude (Phase 1) — but the
    // Agent History entry is provider-agnostic (the transcript zone exists
    // for every provider), so it renders before the gate.
    if (providerId !== "claude") {
        return (
            <Show when={onOpenHistory}>
                <div class="agent-session-row">
                    <button
                        class="agent-session-btn"
                        title="Browse this agent's full recorded history, across sessions"
                        onClick={() => onOpenHistory?.()}
                    >
                        View full history
                    </button>
                </div>
            </Show>
        ) as unknown as JSX.Element;
    }

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

    // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.2: server
    // sets `session:resume_failed` when a `--resume <sid>` was rejected by
    // the CLI (stale/unreachable session, e.g. after a relogin moved this
    // agent to a different config dir) and it silently fell through to a
    // fresh conversation. Unlike `was_interrupted`, there's nothing to
    // "resume on next message" here — the fresh session already started;
    // this banner only discloses that it happened, since the alternative
    // (silence) means the user has no way to know their prior conversation
    // is gone.
    const resumeFailed = (): boolean =>
        (blockAtom()?.meta?.["session:resume_failed"] as boolean | undefined) === true;

    const dismissResumeFailed = async () => {
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", blockId),
                meta: { "session:resume_failed": null } as MetaType,
            });
        } catch (e) {
            console.error("failed to clear resume_failed:", e);
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
                        title="Dismiss this notice — your next message resumes the session either way"
                    >
                        Dismiss
                    </button>
                </div>
            </Show>

            {/* ── Resume-failed disclosure (continuity guarantee §4.2) ── */}
            <Show when={resumeFailed()}>
                <div class="agent-resume-failed-banner">
                    <span class="agent-resume-failed-label">
                        Couldn't resume the previous conversation — started a new one.
                    </span>
                    <button
                        class="agent-session-btn agent-session-btn-dismiss"
                        onClick={dismissResumeFailed}
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
                        title="Compress and archive this session's history to free disk space, then start fresh"
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
                        title="Restore this session's history from the archive"
                    >
                        {restoreBusy() ? "Restoring…" : "Restore"}
                    </button>
                    <button
                        class="agent-session-btn agent-session-btn-export"
                        disabled={exportBusy()}
                        onClick={handleExport}
                        title="Download this session's history as a .jsonl file"
                    >
                        {exportBusy() ? "Exporting…" : "Export"}
                    </button>
                </div>
            </Show>

            {/* ── Session actions (shown when there is history and it's live) ──
                Excludes large sessions: the large-session banner above already
                surfaces an Archive button, so rendering the row here too would
                double it (reagent #1912 P1). */}
            <Show when={lineCount() > 0 && !isArchived() && !isLargeSession()}>
                <div class="agent-session-row">
                    <span class="agent-session-row-label">Session</span>
                    <div class="agent-session-actions">
                        <button
                            class="agent-session-btn agent-session-btn-archive"
                            disabled={archiveBusy()}
                            onClick={handleArchive}
                            title="Compress and archive this session's history to free disk space, then start fresh"
                        >
                            {archiveBusy() ? "Archiving…" : "Archive"}
                        </button>
                        <button
                            class="agent-session-btn agent-session-btn-export"
                            disabled={exportBusy()}
                            onClick={handleExport}
                            title="Download this session's history as a .jsonl file"
                        >
                            {exportBusy() ? "Exporting…" : "Export"}
                        </button>
                    </div>
                </div>
            </Show>

            {/* ── Agent History entry — deliberately UNCONDITIONAL (not
                nested in the session-actions row): archived and 500K+-line
                sessions hide that row but are exactly the states where the
                full-history reader matters most (reagent P1 on PR #2509). */}
            <Show when={onOpenHistory}>
                <div class="agent-session-row">
                    <span class="agent-session-row-label">History</span>
                    <div class="agent-session-actions">
                        <button
                            class="agent-session-btn"
                            title="Browse this agent's full recorded history, across sessions"
                            onClick={() => onOpenHistory?.()}
                        >
                            View full history
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    );
};
