// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSessionNotices — the session banners that used to live in
 * `AgentControlBar`, inside the Shell drawer.
 *
 * They were invisible unless the user happened to open the drawer, which is a
 * terminal toggle: "your next message will resume an interrupted session" and
 * "the previous conversation couldn't be resumed" are conversation-level
 * disclosures, so they now render directly above the composer strip and are
 * always visible while the flag is set. See
 * `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` §3.
 *
 * The Archive/Export actions that shared that component moved to the context
 * stats popover (`AgentSessionStats`); the large-session and archived banners
 * keep their own inline buttons, because each is the disclosure that makes
 * that specific action urgent.
 */

import { createSignal, Show, type JSX } from "solid-js";

import {
    archiveSession,
    clearSessionFlag,
    exportSession,
    isLargeSession,
    isSessionArchived,
    resumeFailed,
    restoreSession,
    sessionArchivedAt,
    sessionLineCount,
    wasInterrupted,
    type BlockAccessor,
} from "../session-actions";

interface AgentSessionNoticesProps {
    blockId: string;
    blockAtom: BlockAccessor;
    providerId: string;
}

export const AgentSessionNotices = (props: AgentSessionNoticesProps): JSX.Element => {
    const [archiveBusy, setArchiveBusy] = createSignal(false);
    const [exportBusy, setExportBusy] = createSignal(false);
    const [restoreBusy, setRestoreBusy] = createSignal(false);

    // Session management is Claude-only, same gate AgentControlBar had.
    const enabled = () => props.providerId === "claude";

    const archivedAtLabel = (): string => {
        const ts = sessionArchivedAt(() => props.blockAtom());
        return ts ? new Date(ts).toLocaleString() : "";
    };

    const run = async (
        busy: () => boolean,
        setBusy: (v: boolean) => void,
        what: string,
        fn: () => Promise<void>
    ) => {
        if (busy()) return;
        setBusy(true);
        try {
            await fn();
        } catch (e) {
            console.error(`session:${what} failed:`, e);
        } finally {
            setBusy(false);
        }
    };

    const dismiss = async (key: string) => {
        try {
            await clearSessionFlag(props.blockId, key);
        } catch (e) {
            console.error(`failed to clear ${key}:`, e);
        }
    };

    return (
        <Show when={enabled()}>
            <div class="agent-session-notices">
                {/* ── Interrupted-session recovery banner (4.2 multi-day continuity) ── */}
                <Show when={wasInterrupted(() => props.blockAtom())}>
                    <div class="agent-interrupted-banner">
                        <span class="agent-interrupted-label">
                            Session was interrupted by a restart. Your next message will resume it.
                        </span>
                        <button
                            class="agent-session-btn agent-session-btn-dismiss"
                            onClick={() => void dismiss("session:was_interrupted")}
                            title="Dismiss this notice — your next message resumes the session either way"
                        >
                            Dismiss
                        </button>
                    </div>
                </Show>

                {/* ── Resume-failed disclosure (continuity guarantee §4.2) ── */}
                <Show when={resumeFailed(() => props.blockAtom())}>
                    <div class="agent-resume-failed-banner">
                        <span class="agent-resume-failed-label">
                            Couldn't resume the previous conversation — started a new one.
                        </span>
                        <button
                            class="agent-session-btn agent-session-btn-dismiss"
                            onClick={() => void dismiss("session:resume_failed")}
                            title="Dismiss this notice"
                        >
                            Dismiss
                        </button>
                    </div>
                </Show>

                {/* ── Large session warning (4.1 graceful degradation) ── */}
                <Show when={isLargeSession(() => props.blockAtom())}>
                    <div class="agent-large-session-banner">
                        <span class="agent-large-session-label">
                            Session has {sessionLineCount(() => props.blockAtom()).toLocaleString()} lines.
                            Consider archiving to free disk space.
                        </span>
                        <button
                            class="agent-session-btn agent-session-btn-archive"
                            disabled={archiveBusy()}
                            onClick={() =>
                                void run(archiveBusy, setArchiveBusy, "archive", () =>
                                    archiveSession(props.blockId)
                                )
                            }
                            title="Compress and archive this session's history to free disk space, then start fresh"
                        >
                            {archiveBusy() ? "Archiving…" : "Archive"}
                        </button>
                    </div>
                </Show>

                {/* ── Archived badge (shown when session is archived) ── */}
                <Show when={isSessionArchived(() => props.blockAtom())}>
                    <div class="agent-archived-banner">
                        <span class="agent-archived-label" title={`Archived at ${archivedAtLabel()}`}>
                            Archived
                        </span>
                        <button
                            class="agent-session-btn agent-session-btn-restore"
                            disabled={restoreBusy()}
                            onClick={() =>
                                void run(restoreBusy, setRestoreBusy, "restore", () =>
                                    restoreSession(props.blockId)
                                )
                            }
                            title="Restore this session's history from the archive"
                        >
                            {restoreBusy() ? "Restoring…" : "Restore"}
                        </button>
                        <button
                            class="agent-session-btn agent-session-btn-export"
                            disabled={exportBusy()}
                            onClick={() =>
                                void run(exportBusy, setExportBusy, "export", () =>
                                    exportSession(props.blockId)
                                )
                            }
                            title="Download this session's history as a .jsonl file"
                        >
                            {exportBusy() ? "Exporting…" : "Export"}
                        </button>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

AgentSessionNotices.displayName = "AgentSessionNotices";
