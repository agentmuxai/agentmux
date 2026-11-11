// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The panels docked between the transcript and the composer, in parts (AgentBottomPanels.tsx): the login, the
 * request stack, the queue, the recovery banners, the activity dock, the turn's status and the session banners.
 * Each part is a fragment, not a wrapper, so every panel stays a direct flex child of `.agent-view-zoomed`; each
 * reads the panels' props and the derived values in agent-bottom-panels-state.ts.
 */

import { isStopping } from "@/app/store/agent-pane-state/types";
import { plausibleReading } from "@/app/store/agent-pane-state/context-reading";
import { readPaneSummary } from "@/app/store/swarm-line";
import { useTick } from "@/app/hook/useTick";
import { clockTime } from "@/store/muxbus-delivery";
import { Show, type JSX } from "solid-js";
import { SensitiveBanner } from "../sensitive/SensitiveBanner";
import { ShutdownPendingBanner } from "../shutdown/ShutdownPendingBanner";
import { HeldInputPanel } from "../offline/HeldInputPanel";
import { PaneRequestDock } from "../pane-requests/PaneRequestDock";
import { compactionModelKey } from "../compaction-estimate";
import { ActivityDock } from "./ActivityDock";
import { AgentCredentialsRevokedChip } from "./AgentCredentialsRevokedChip";
import { AgentDecisionPanel } from "./AgentDecisionPanel";
import { AgentDisconnectedBanner } from "./AgentDisconnectedBanner";
import { DesktopDrivingBar } from "./DesktopDrivingBar";
import { AgentAuthPanel } from "./AgentDocumentView";
import { AgentWorkingRow } from "./working-row";
import { ApiRetryRow } from "./ApiRetryRow";
import { AgentQuestionPanel } from "./AgentQuestionPanel";
import { AgentSessionNotices } from "./AgentSessionNotices";
import { AgentStoppedBanner } from "./AgentStoppedBanner";
import { ForkProviderFallbackBanner } from "./ForkProviderFallbackBanner";
import { HistoryLoadFailedBanner } from "./HistoryLoadFailedBanner";
import { PaneRow } from "./PaneRow";
import { TurnAwayDigest } from "./TurnAwayDigest";
import { PendingMessagesPanel } from "./PendingMessagesPanel";
import type { AgentBottomPanelsProps, BottomPanelsState } from "./agent-bottom-panels-state";

type Part = { p: AgentBottomPanelsProps; t: BottomPanelsState };

/** The desktop-driving bar and the login panel. */
export function LoginPanels(part: Part): JSX.Element {
    const { p } = part;
    return (
        <>
            {/* An agent may control a native app: say so, and offer Stop. */}
            <DesktopDrivingBar blockId={p.blockId} />
            {/* Login UI — bottom-docked like AgentDecisionPanel/
                AgentQuestionPanel below, not inside the scrollable document.
                See #2429 follow-up: it used to render inside
                AgentDocumentView's header slot, which pinned it to the top
                of the scroll area. */}
            <AgentAuthPanel
                authUrl={p.status.authUrl}
                authNotice={p.status.authNotice}
                onDismissAuthNotice={() => p.status.setAuthNotice(null)}
                onCancelLogin={p.status.cancelLogin}
                onUseTerminal={p.status.useTerminalInstead}
                authProviderId={p.authProviderId}
                launchPhase={p.status.launchPhase}
            />

            {/* The separate blue "Log in" bar that used to render here on
                `status.canRetry()` is GONE — it was a second CTA for the
                identical action (relogin()) as the failure row below, and the
                two could be on screen simultaneously (a pane reopened after an
                auth failure seeds the row from persisted block meta while the
                mount-time launch flow independently sets canRetry). That case
                now raises a synthetic `turnAttempted: false` auth failure
                instead (failure/useAuthHealth.ts), so it renders through the
                one shared row, labelled "Log in" and still passing
                `retryAfterLogin: false`.

                `status.canRetry()` itself is DELIBERATELY still live — it is
                not only a display gate: useAgentCommands reads it to fast-fail
                sends into an unauthenticated agent, and /login reads it too.
                Deleting the signal along with this bar would silently re-open
                that hole. See
                docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md. */}
        </>
    );
}

/** The permission decision, the question and the requests raised for this pane, in the one wrapper that caps them together. */
export function RequestStack(part: Part): JSX.Element {
    const { p, t } = part;
    return (
        <>
            {/* Permission decision panel — surfaced when one or more
                tool calls are gated by the CLI awaiting user approval.
                Sits above the queue so it can't be missed. The panel
                renders nothing when no ToolNode is in pending_approval.
                Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §5. */}
            <div class="agent-request-stack">
                <AgentDecisionPanel
                    pending={p.pendingDecisions}
                    onDecide={p.onDecide}
                    onDefer={t.decisionMinimized}
                />

                {/* AskUserQuestion panel — surfaced when a tool call is in
                    `awaiting_answer` (the agent asked the user a structured
                    question and is blocked on the answer). Submitting delivers a
                    tool_result over the persistent controller's stdin. Cancel
                    (button / Escape) is a REAL protocol-level decline via
                    `handleCancel`, not a UI-only dismiss — replaces the old
                    "Answer later" minimize, which never told the agent anything.
                    Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
                    docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md. */}
                <AgentQuestionPanel
                    blockId={p.blockId}
                    pending={p.pendingQuestions}
                    onAnswer={p.onAnswer}
                    onCancel={p.onCancel}
                />
                {/* Requests raised for this pane, as cards (SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.1). */}
                <PaneRequestDock blockId={p.blockId} />
            </div>
        </>
    );
}

/** The messages queued while the agent is busy. */
export function PendingQueue(part: Part): JSX.Element {
    const { p } = part;
    return (
        <>
            {/* Queue sits directly below the feed so the user's newly-
                typed message lands next to the live conversation it's
                queued against. Previously lived below the activity log;
                repositioning per SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.
                No "Send now" affordance — Esc on an empty composer delivers
                a queued message immediately instead (mirrors Claude Code
                CLI). See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md. */}
            <PendingMessagesPanel pendingMessages={p.pendingMessages} />
        </>
    );
}

/** The chips and banners for a revoked or stopped session, the failure row, a dropped stream, a fork's fallback and a history that did not load. */
export function RecoveryBanners(part: Part): JSX.Element {
    const { p, t } = part;
    return (
        <>
            {/* PR F — Disconnected banner. Visible when the stream
                tore down while a turn was in flight (kind=Disconnected).
                Sits above the status line so the working spinner (which
                is already suppressed because `isWorking(Disconnected) =
                false`) doesn't overlay the disconnect message. The
                Reconnect button re-subscribes; the reducer's
                `StreamSubscribe` arm clears the phase to Idle. Spec
                docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md
                §6.4. */}
            {/* Credentials-revoked disclosure chip — appears when an identity
                account this agent was linked to is deleted (or unlinked)
                while the pane is live. Honest wording: the running process
                still holds working tokens until restarted; enforcement lands
                at the next spawn (layer 3).
                SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §3. */}
            <AgentCredentialsRevokedChip agentId={p.agentId} flowRunning={p.status.flowRunning} />
            {/* Something other than the user asked to shut this agent down:
                30 s to keep it (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). */}
            <ShutdownPendingBanner blockId={p.blockId} agentId={p.agentId} agentName={p.agentName} />
            {/* A sensitive operation the super agent approved: the person can veto it. */}
            <SensitiveBanner />
            {/* Failure-recovery row — per-error-class actions + auto-retry,
                rendered through the shared PaneRow accessory primitive.
                SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16. */}
            <Show when={p.failureRow()}>
                {(row) => (
                    <PaneRow
                        sigil={row().sigil}
                        title={row().title}
                        meta={row().meta}
                        accent={row().accent}
                        actions={row().actions}
                        expanded={row().expanded}
                    >
                        <div class="agent-failure-detail">
                            <div>{row().detail}</div>
                            <Show when={row().stderrTail}>
                                <pre class="agent-failure-stderr">{row().stderrTail}</pre>
                            </Show>
                        </div>
                    </PaneRow>
                )}
            </Show>
            <AgentDisconnectedBanner phase={(() => p.paneModel.state.turnPhase)} onReconnect={t.reconnect} />
            {/* Non-Claude quick-fork fallback note — SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
                §4.4. Set once on the new block's meta right after a fork
                lands with no `--fork-session` support; stays for the pane's
                lifetime, no dismiss button (quick-fork.ts). */}
            <ForkProviderFallbackBanner meta={() => p.block()?.meta} />
            {/* The earlier messages couldn't be loaded (InitFailed): say so, not an empty pane. */}
            <HistoryLoadFailedBanner phase={() => p.paneModel.state.initPhase} />
        </>
    );
}

/** The pinned activity dock. */
export function ActivityDockRow(part: Part): JSX.Element {
    const { p } = part;
    return (
        <>
            {/* Pinned activity dock — long-running shells (and later crons /
                subagents) sit just above the composer so task status is adjacent
                to where the user's attention already is. Moved from the top per
                SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20. */}
            <ActivityDock
                documentNodes={p.paneModel.document}
                blockId={p.blockId}
                backgroundTasksAtom={p.backgroundTasksAtom}
            />
        </>
    );
}

/** The working row, while it is visible. */
export function WorkingRowAnchor(part: Part): JSX.Element {
    const { p, t } = part;
    // The agent's own status line stops counting after an hour (`readPaneSummary`), with no meta update to say so.
    const minute = useTick(60_000);
    return (
        <>
            <div class="agent-working-row-anchor">
                <Show when={p.workingRowVisible()}>
                    <AgentWorkingRow
                        loading={p.workingRowLoading()}
                        stopping={isStopping(p.paneModel.state.turnPhase)}
                        activitySummary={(minute(), readPaneSummary(p.block()?.meta))}
                        turnTokens={p.paneModel.state.turnTokens}
                        turnLedger={p.paneModel.state.turnLedger}
                        turnCarry={p.paneModel.state.turnCarry}
                        activity={p.paneModel.state.activity}
                        needsYou={t.needsYou()}
                        sessionStats={p.paneModel.state.sessionStats}
                        launchPhase={p.status.launchPhase()}
                        onCancelLogin={p.status.cancelLogin}
                        hasAuthUrl={!!p.status.authUrl()}
                        waitingReason={t.waitingReason()}
                        retryAtMs={t.retryAtMs()}
                        compacting={p.paneModel.state.compacting}
                        compactionContextTokens={plausibleReading(p.paneModel.state.context)?.tokens ?? null}
                        compactionModel={compactionModelKey(p.block()?.meta, p.paneModel.state.lastContextModel)}
                        reconnecting={p.paneModel.state.reconnecting}
                    />
                </Show>
            </div>
        </>
    );
}

/** What the turn is doing: the digest of turns missed, an API retry, input held offline, and the working row. */
export function TurnStatus(part: Part): JSX.Element {
    const { p, t } = part;
    return (
        <>
            {/* Working indicator — the turn's own status, so it sits directly
                above the composer, with the dock's long-running tasks stacked
                above it. Reads bottom-up as narrowing scope: what's running in
                the background (dock) → what this turn is doing right now
                (here) → where you type.

                Normal-flow row, not the overlay it used to be — see the
                .agent-document-scroll-region comment in agent-view.tsx for what that
                change removed. When it appears or disappears it changes the
                scroll region's clientHeight, which
                AgentDocumentVirtualList's clientHeight ResizeObserver already
                re-pins on; that observer was written for exactly this family
                of normal-flow siblings (the retry bar, decision/question
                panels, PendingMessagesPanel), so the row simply joins them
                rather than needing its own tracked height signal.

                Shows spinner + elapsed while loading, "✓ Worked · Ns" on
                completion. Acts as a visual turn delimiter; stays until the
                next message is sent.
                See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §2 and
                SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md. */}
            {/* Turns you didn't start that finished while you were away. */}
            <TurnAwayDigest
                blockId={p.blockId}
                looking={t.looking}
                busy={p.workingRowLoading}
            />
            <ApiRetryRow retry={p.offline.apiRetry()} onStop={p.onStop} />
            <HeldInputPanel
                held={p.offline.held()}
                notices={p.offline.notices()}
                onSendNow={() => void p.offline.sendNow()}
                onCancel={p.offline.cancel}
                onDismissNotice={p.offline.dismissNotice}
                clockTime={clockTime}
            />
            <WorkingRowAnchor p={p} t={t} />
        </>
    );
}

/** The session banners and the note that the agent was stopped. */
export function SessionBanners(part: Part): JSX.Element {
    const { p } = part;
    return (
        <>
            {/* Session banners (interrupted / resume-failed / large /
                archived). Above the strip, NOT inside the Shell drawer where
                they used to live: a conversation-level disclosure can't be
                gated behind a terminal toggle — see
                SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3. */}
            <AgentSessionNotices
                blockId={p.blockId}
                blockAtom={p.block}
                providerId={p.providerId}
            />
            {/* An agent that was stopped keeps its pane and conversation; Start runs it again. */}
            <AgentStoppedBanner block={p.block} agentId={p.agentId} />
        </>
    );
}
