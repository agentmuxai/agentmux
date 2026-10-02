// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 6).

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { readSwarmSummary } from "@/app/store/activitySummary";
import { isStopping } from "@/app/store/agent-pane-state/types";
import { Show, type Accessor, type ComponentProps } from "solid-js";
import type { UseAgentFailureResult } from "../hooks/useAgentFailure";
import type { UseAgentControllerStatus } from "../hooks/useAgentControllerStatus";
import type { useShellLogBridge } from "../hooks/useShellLogBridge";
import { ShutdownPendingBanner } from "../shutdown/ShutdownPendingBanner";
import { ActivityDock } from "./ActivityDock";
import { AgentCredentialsRevokedChip } from "./AgentCredentialsRevokedChip";
import { AgentDecisionPanel } from "./AgentDecisionPanel";
import { AgentDisconnectedBanner } from "./AgentDisconnectedBanner";
import { AgentAuthPanel } from "./AgentDocumentView";
import { AgentWorkingRow } from "./AgentFooter";
import { AgentQuestionPanel } from "./AgentQuestionPanel";
import { AgentSessionNotices } from "./AgentSessionNotices";
import { ForkProviderFallbackBanner } from "./ForkProviderFallbackBanner";
import { PaneRow } from "./PaneRow";
import { PendingMessagesPanel } from "./PendingMessagesPanel";

type DecisionProps = ComponentProps<typeof AgentDecisionPanel>;
type QuestionProps = ComponentProps<typeof AgentQuestionPanel>;

/**
 * Everything docked between the transcript and the composer strip: login,
 * decision and question panels, the queue, the recovery banners, the activity
 * dock, the working row and the session notices.
 *
 * Renders a fragment, not a wrapper: each panel stays a direct flex child of
 * `.agent-view-zoomed`, which the layout and AgentDocumentVirtualList's
 * clientHeight re-pin rely on (§3.3).
 */
export const AgentBottomPanels = (props: {
    blockId: string;
    agentId: string;
    agentName: string;
    /** For the login panel: the provider's id, else its key. */
    authProviderId: string;
    /** For the session notices: the provider's id, else "". */
    providerId: string;
    block: ComponentProps<typeof AgentSessionNotices>["blockAtom"];
    paneModel: AgentPaneModel;
    status: UseAgentControllerStatus;
    log: ReturnType<typeof useShellLogBridge>["log"];
    pendingDecisions: DecisionProps["pending"];
    onDecide: DecisionProps["onDecide"];
    pendingQuestions: QuestionProps["pending"];
    onAnswer: QuestionProps["onAnswer"];
    onCancel: QuestionProps["onCancel"];
    hidden: QuestionProps["isDormant"];
    pendingMessages: ComponentProps<typeof PendingMessagesPanel>["pendingMessages"];
    failureRow: UseAgentFailureResult["row"];
    backgroundTasksAtom: ComponentProps<typeof ActivityDock>["backgroundTasksAtom"];
    workingRowVisible: Accessor<boolean>;
    workingRowLoading: Accessor<boolean>;
}) => (
    <>
        {/* Login UI — bottom-docked like AgentDecisionPanel/
            AgentQuestionPanel below, not inside the scrollable document.
            See #2429 follow-up: it used to render inside
            AgentDocumentView's header slot, which pinned it to the top
            of the scroll area. */}
        <AgentAuthPanel
            authUrl={props.status.authUrl}
            authNotice={props.status.authNotice}
            onDismissAuthNotice={() => props.status.setAuthNotice(null)}
            onCancelLogin={props.status.cancelLogin}
            onUseTerminal={props.status.useTerminalInstead}
            authProviderId={props.authProviderId}
            launchPhase={props.status.launchPhase}
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

        {/* Permission decision panel — surfaced when one or more
            tool calls are gated by the CLI awaiting user approval.
            Sits above the queue so it can't be missed. The panel
            renders nothing when no ToolNode is in pending_approval.
            Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §5. */}
        <AgentDecisionPanel
            pending={props.pendingDecisions}
            onDecide={props.onDecide}
            onDefer={() => {
                // Logging only — the panel itself manages the
                // minimized state (per doc §7 + §4.3) so the
                // prompt remains reachable.
                props.log("agent", "Decision minimized");
            }}
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
            pending={props.pendingQuestions}
            onAnswer={props.onAnswer}
            onCancel={props.onCancel}
            isDormant={props.hidden}
        />

        {/* Queue sits directly below the feed so the user's newly-
            typed message lands next to the live conversation it's
            queued against. Previously lived below the activity log;
            repositioning per SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.
            No "Send now" affordance — Esc on an empty composer delivers
            a queued message immediately instead (mirrors Claude Code
            CLI). See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md. */}
        <PendingMessagesPanel pendingMessages={props.pendingMessages} />

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
        <AgentCredentialsRevokedChip agentId={props.agentId} />
        {/* Something other than the user asked to shut this agent down:
            15 s to keep it (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). */}
        <ShutdownPendingBanner blockId={props.blockId} agentId={props.agentId} agentName={props.agentName} />
        {/* Failure-recovery row — per-error-class actions + auto-retry,
            rendered through the shared PaneRow accessory primitive.
            SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16. */}
        <Show when={props.failureRow()}>
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
        <AgentDisconnectedBanner
            phase={(() => props.paneModel.state.turnPhase)}
            onReconnect={() => {
                // Standard stream-reconnect path: dispatch
                // `StreamSubscribe` against the live pane. If the
                // backend has auto-reconnected between render and
                // click, the second subscribe is harmless — the
                // reducer's Disconnected→Idle transition is the
                // same regardless of who calls it.
                props.paneModel.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "user");
            }}
        />
        {/* Non-Claude quick-fork fallback note — SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
            §4.4. Set once on the new block's meta right after a fork
            lands with no `--fork-session` support; stays for the pane's
            lifetime, no dismiss button (quick-fork.ts). */}
        <ForkProviderFallbackBanner meta={() => props.block()?.meta} />

        {/* Pinned activity dock — long-running shells (and later crons /
            subagents) sit just above the composer so task status is adjacent
            to where the user's attention already is. Moved from the top per
            SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20. */}
        <ActivityDock
            documentNodes={props.paneModel.document}
            blockId={props.blockId}
            backgroundTasksAtom={props.backgroundTasksAtom}
        />

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
        <div class="agent-working-row-anchor">
            <Show when={props.workingRowVisible()}>
                <AgentWorkingRow
                    loading={props.workingRowLoading()}
                    stopping={isStopping(props.paneModel.state.turnPhase)}
                    activitySummary={readSwarmSummary(props.block()?.meta)}
                    turnTokens={props.paneModel.state.turnTokens}
                    sessionStats={props.paneModel.state.sessionStats}
                    launchPhase={props.status.launchPhase()}
                    onCancelLogin={props.status.cancelLogin}
                    hasAuthUrl={!!props.status.authUrl()}
                    waitingReason={(() => {
                        const phase = props.paneModel.state.turnPhase;
                        return phase.kind === "Streaming" ? (phase.waitingReason ?? null) : null;
                    })()}
                    retryAfterMs={(() => {
                        const phase = props.paneModel.state.turnPhase;
                        return phase.kind === "Streaming" ? (phase.retryAfterMs ?? null) : null;
                    })()}
                    compacting={props.paneModel.state.compacting}
                    compactionContextTokens={props.paneModel.state.lastContextTokens}
                    reconnecting={props.paneModel.state.reconnecting}
                />
            </Show>
        </div>

        {/* Session banners (interrupted / resume-failed / large /
            archived). Above the strip, NOT inside the Shell drawer where
            they used to live: a conversation-level disclosure can't be
            gated behind a terminal toggle — see
            SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3. */}
        <AgentSessionNotices
            blockId={props.blockId}
            blockAtom={props.block}
            providerId={props.providerId}
        />
    </>
);
