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

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";

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
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";

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
                            {t("session.notice.interrupted")}
                        </span>
                        <Button
                            density="compact"
                            onClick={() => void dismiss("session:was_interrupted")}
                            title={t("session.notice.dismissResumeTip")}
                        >
                            {t("session.notice.dismiss")}
                        </Button>
                    </div>
                </Show>

                {/* ── Resume-failed disclosure (continuity guarantee §4.2) ── */}
                <Show when={resumeFailed(() => props.blockAtom())}>
                    <ResumeFailedBanner onDismiss={() => void dismiss("session:resume_failed")} />
                </Show>

                {/* ── Large session warning (4.1 graceful degradation) ── */}
                <Show when={isLargeSession(() => props.blockAtom())}>
                    <div class="agent-large-session-banner">
                        <span class="agent-large-session-label">
                            {t("session.notice.large", { lines: sessionLineCount(() => props.blockAtom()).toLocaleString() })}
                        </span>
                        <Button
                            density="compact"
                            disabled={archiveBusy()}
                            onClick={() =>
                                void run(archiveBusy, setArchiveBusy, "archive", () =>
                                    archiveSession(props.blockId)
                                )
                            }
                            title={t("session.notice.archiveTip")}
                        >
                            {archiveBusy() ? t("session.notice.archiving") : t("session.notice.archive")}
                        </Button>
                    </div>
                </Show>

                {/* ── Archived badge (shown when session is archived) ── */}
                <Show when={isSessionArchived(() => props.blockAtom())}>
                    <div class="agent-archived-banner">
                        <span class="agent-archived-label" {...tip(t("session.notice.archivedAt", { when: archivedAtLabel() }))}>
                            {t("session.notice.archived")}
                        </span>
                        <Button
                            density="compact"
                            disabled={restoreBusy()}
                            onClick={() =>
                                void run(restoreBusy, setRestoreBusy, "restore", () =>
                                    restoreSession(props.blockId)
                                )
                            }
                            title={t("session.notice.restoreTip")}
                        >
                            {restoreBusy() ? t("session.notice.restoring") : t("session.notice.restore")}
                        </Button>
                        <Button
                            density="compact"
                            disabled={exportBusy()}
                            onClick={() =>
                                void run(exportBusy, setExportBusy, "export", () =>
                                    exportSession(props.blockId)
                                )
                            }
                            title={t("session.notice.exportTip")}
                        >
                            {exportBusy() ? t("session.notice.exporting") : t("session.notice.export")}
                        </Button>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

AgentSessionNotices.displayName = "AgentSessionNotices";

/** Seconds the failed-resume banner stays up before dismissing itself. */
const RESUME_FAILED_AUTO_DISMISS_S = 15;

/**
 * The failed-resume banner dismisses itself after a visible countdown: it's a
 * one-time disclosure, and the transcript's own session-outcome divider keeps
 * the permanent record. Dismissing (either way) clears the flag, same as the
 * button always did. Mounted by the `Show` above, so a fresh failure gets a
 * fresh countdown.
 */
const ResumeFailedBanner = (props: { onDismiss: () => void }): JSX.Element => {
    const [remaining, setRemaining] = createSignal(RESUME_FAILED_AUTO_DISMISS_S);
    let done = false;
    const finish = () => {
        if (done) return;
        done = true;
        clearInterval(timer);
        props.onDismiss();
    };
    const timer = setInterval(() => {
        const next = remaining() - 1;
        setRemaining(next);
        if (next <= 0) finish();
    }, 1000);
    onCleanup(() => clearInterval(timer));

    return (
        <div class="agent-resume-failed-banner">
            <span class="agent-resume-failed-label">
                {t("session.notice.resumeFailed")}
            </span>
            <span class="agent-resume-failed-countdown">{remaining()}s</span>
            <Button
                density="compact"
                onClick={finish}
                title={t("session.notice.dismissTip")}
            >
                {t("session.notice.dismiss")}
            </Button>
        </div>
    );
};
