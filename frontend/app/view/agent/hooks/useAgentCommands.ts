// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentCommands — the agent pane's user-driven handlers: sending
 * messages (with `!` and slash-command intercepts) and returning to the
 * agent picker. Extracted from agent-view.tsx, step 12 of
 * docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Client-side slash commands run locally (`/login` runs the GUI OAuth flow
 * and shows its URL via `setAuthUrl`; `/clear` resets the document). All
 * other messages go to the backend via `RpcApi.AgentInputCommand`, after
 * `cmd:args` is updated so runtime flags (permission mode, model, effort)
 * apply to this turn.
 */

import { type Accessor, createMemo, createSignal, onCleanup } from "solid-js";
import { trail } from "@/log/render-trail";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { isAuthFailure, workingFromPhase, type PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { buildPaneArgs, getRuntimeConfig } from "../buildRuntimeArgs";
import { PROVIDER_FLAGS_META_KEY } from "../launch-args";
import { busyInputFromState, paneBusyForInput } from "../working-indicator";
import { dispatchSlashCommand } from "../commands/dispatch";
import { buildRegistry } from "../commands/registry";
import type { SlashCommand, SlashCommandContext, SlashPickerSpec } from "../commands/types";
import { parseBangCommand } from "../bang-command";
import type { ProviderDefinition } from "../providers";
import type { DocumentNode } from "../types";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { LogFn } from "./useAgentControllerStatus";
import { setBlockMeta } from "@/app/store/block-meta";

/**
 * How long a pending message can sit unacknowledged before the reducer
 * removes it. 30s leaves margin over the typical <2s local turnaround for
 * transient hiccups (#728).
 */
const PENDING_TIMEOUT_MS = 30_000;

export interface UseAgentCommandsOptions {
    blockId: string;
    /**
     * Per-pane model from `registerPane`; `model.dispatchPane` is safe
     * against post-unmount races. See `agent-pane-model.ts`.
     */
    model: AgentPaneModel;
    block: Accessor<{ meta?: Record<string, any> } | undefined>;
    provider: Accessor<ProviderDefinition | undefined>;
    /** Reactive accessor for the pane document nodes (`model.document`). */
    documentNodes: Accessor<DocumentNode[]>;
    /**
     * The view's launch/auth-activity flag, so the §2.3a eager flush below
     * can evaluate the WHOLE `paneBusyForInput` predicate. Defaults to
     * `false` (non-view callers, i.e. tests, are never mid-launch); safe
     * because that flush also requires a Streaming turn with accepted
     * background work, which a launching pane never has.
     */
    showingLaunchActivity?: Accessor<boolean>;
    log: LogFn;
    setAuthUrl: (url: string | null) => void;
    /**
     * True when the pane already knows it is logged out
     * (`useAgentControllerStatus`'s `canRetry`: no turn ever attempted, or
     * a recovery attempt just failed). `deliverToBackend` fails fast on it
     * instead of spawning a CLI that only reports "Not logged in" after a
     * network round-trip.
     */
    canRetry: Accessor<boolean>;
    /**
     * True while a relogin()/loginViaTerminal() recovery attempt is in
     * flight. `canRetry` clears the instant "Log in" is clicked, before the
     * attempt resolves, so without this a message sent mid-attempt reaches
     * a credential the pane still distrusts (#2338).
     */
    loginWaiting: Accessor<boolean>;
    /** The error box above the composer — the same surface as
     *  `useAgentControllerStatus`'s recovery notices, so the fast-fail
     *  reads like every other auth-recovery message. */
    setAuthNotice: (notice: string | null) => void;
    /** For /login to clear a stale `canRetry` on success — see
     *  `SlashCommandContext.notifyControllerHealthy`. */
    notifyControllerHealthy: () => void;
    /** For /login to restart a running controller onto the new
     *  credential — see `SlashCommandContext.forceControllerRefresh`. */
    forceControllerRefresh: () => Promise<boolean>;
    /** For /login to count its own poll toward `loginWaiting` — see
     *  `SlashCommandContext.beginRecoveryFlow`. */
    beginRecoveryFlow: () => void;
    /** Pairs with `beginRecoveryFlow` — see its doc comment. */
    endRecoveryFlow: () => void;
    /** For /login's poll to see a cancel made through the shared
     *  AuthUrlBox — see `SlashCommandContext.isCancelled`. */
    isCancelled: () => boolean;
    /** For /login to clear a cancel flag left by an earlier attempt — see
     *  `SlashCommandContext.resetCancelled`. */
    resetCancelled: () => void;
    /**
     * The last confirmed backend `turn_active` from live controllerstatus
     * events (`turn-confirmation.ts`'s `wasTurnActive`); false until one
     * says otherwise. ORed into `isTurnActive()` because a premature
     * per-round `session_end` can demote `turnPhase` to "Done" while the
     * backend is still working, and /login would restart a busy
     * controller (#2338).
     */
    isBackendTurnActive: () => boolean;
    /**
     * True only when the backend positively confirmed the turn ended
     * (`wasTurnActive === false`) — NOT `!isBackendTurnActive()`, which is
     * also true when never confirmed. Gates `flushPendingControllerRefresh`'s
     * destructive restart: "unknown" must mean "not safe", or a pane that
     * mounted mid-turn could kill a turn it never saw end (#2338).
     */
    isBackendTurnConfirmedIdle: () => boolean;
    /** The model-level backToPicker action, shared with the pane-frame
     *  header button so it lives in one place (AgentViewModel).
     *  See SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md item #8. */
    backToPicker: () => Promise<void>;
    /**
     * Called on the next animation frame after `sendMessage` queues a user
     * message; wired to the document's `scrollToBottomFn` so the message is
     * visible even if `stickToBottom` was switched off while the composer grew.
     * See SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md item #1.
     */
    onSent?: () => void;
    /**
     * Queue of messages sent to the backend but not yet accepted.
     * `sendMessage` appends here (instead of directly to the document)
     * and `useAgentStream` removes entries on `agent-message-accepted`,
     * promoting them into the document at that moment.
     */
    pendingMessages?: Accessor<import("../state").PendingMessage[]>;
    /**
     * `quick-fork.ts`'s `quickForkAgent` bound to this pane, for `/fork`
     * (the same action as the "Quick Fork" context-menu item). Optional so
     * tests needn't supply it; `buildCommandContext` defaults it to a no-op
     * resolving `false`.
     */
    quickFork?: () => Promise<boolean>;
    /**
     * `btw.ts`'s `askSideQuestion` bound to this pane, for `/btw` to fire
     * the backend request. This hook, not `btw.ts`, owns the `btwOverlay`
     * signal (see `SlashCommandContext.askSideQuestion`). Optional; defaults
     * to a stub resolving `{ requestId: "" }`.
     */
    askSideQuestion?: (question: string) => Promise<{ requestId: string }>;
}

/**
 * State of the floating `/btw` overlay (`components/BtwOverlay.tsx`). Owned
 * by this hook, like `pickerSpec`/`helpVisible`; the view only reads it.
 */
export interface BtwOverlayState {
    /**
     * Minted per ask, before any `requestId` exists. Asks can share text,
     * and a second ask reuses the mounted `BtwOverlay` (`btwOverlay()` never
     * goes falsy in between), so `BtwOverlay` keys its local-state reset on
     * this; otherwise the previous ask's answer leaks into the new one.
     */
    askId: number;
    /** The question exactly as typed, shown immediately on open — before
     *  `requestId` (and therefore any streamed answer) is even known. */
    question: string;
    /** `null` until the backend accepts the request; the overlay's MPS
     *  subscription (`block:<blockId>:btw:<requestId>`) can't start before. */
    requestId: string | null;
    /** Set if the backend request itself failed (not a streaming-answer
     *  error, which the overlay's own subscription handles separately).
     *  Non-null and `requestId` non-null never coexist. */
    error: string | null;
}

export interface UseAgentCommands {
    /** Send a user message. Slash commands are intercepted via the registry.
     *  `wasAlreadyWorking`: a turn was in flight before the caller
     *  dispatched TurnStart — drives PendingMessage.enqueuedWhileBusy.
     *  `authFailureToPreserve`: the caller's pre-TurnStart read of an
     *  "auth"-classified `state.failure` (else null). TurnStart clears it,
     *  so only the caller can capture it; when set, the fast-fail guard
     *  rejects the send and re-dispatches it so the banner and its login
     *  actions reappear (#2338). */
    sendMessage: (
        message: string,
        wasAlreadyWorking?: boolean,
        authFailureToPreserve?: PaneFailure | null,
        attachments?: AttachmentRef[],
    ) => Promise<void>;
    /**
     * Deliver any messages held while the agent was busy (the "send now"
     * queue). Called by the agent view at the next tool-call boundary (or
     * turn end) so a queued message lands just before the agent's next tool
     * call — it finishes its current train of thought, then picks it up.
     */
    flushHeldMessages: () => Promise<void>;
    /**
     * Pop the most-recently queued (held, not-yet-delivered) message off the
     * "send now" queue and return its text so the composer can restore it —
     * the Claude-Code-CLI ArrowUp "un-queue" gesture. Returns null if the
     * queue is empty. The message was never sent, so this is a true un-send.
     */
    recallLatestHeld: () => { text: string; attachments?: AttachmentRef[] } | null;
    /** True when there are queued-while-busy messages awaiting delivery. */
    hasHeldMessages: () => boolean;
    /** Return to the agent picker by clearing the agent-identity meta keys. */
    back: () => Promise<void>;
    /**
     * Send SIGINT to the currently running agent CLI process. Invoked
     * from the composer's Esc handler when the textarea is empty —
     * equivalent to Ctrl+C in a terminal. Silently no-ops if the
     * controller rejects the signal (e.g. no process running).
     * See SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md item #9.
     */
    stopAgent: () => void;
    /**
     * Inline picker state. Non-null when a slash command needs to
     * resolve a required enum/dynamic arg via the picker UI. The
     * AgentPresentationView reads this to decide whether to render
     * <SlashCommandPicker /> above the composer.
     */
    pickerSpec: Accessor<SlashPickerSpec | null>;
    /** Resolve the picker promise with the chosen value. */
    resolvePicker: (value: string) => void;
    /** Reject the picker promise (Esc / dismiss). */
    dismissPicker: () => void;
    /**
     * Autocomplete completions for the composer. Returns commands
     * available in the current context whose name or alias starts
     * with the given prefix (no leading slash). Sorted by category
     * then name. Consumed by AgentFooter to render the inline
     * autocomplete dropdown.
     */
    completions: (prefix: string) => SlashCommand[];
    /**
     * Help panel state. /help sets this to true via ctx.openHelp;
     * AgentPresentationView reads it to mount <SlashHelpPanel />.
     */
    helpVisible: Accessor<boolean>;
    /** Close the help panel (Esc / close button / row click). */
    closeHelp: () => void;
    /**
     * Every command currently available in this pane (post-availability
     * filter). Consumed by SlashHelpPanel to render the grouped list.
     */
    availableCommands: () => SlashCommand[];
    /**
     * `/btw` overlay state, non-null while it shows (agent-view.tsx mounts
     * `<BtwOverlay />` on it). Set as soon as `askSideQuestion` is called,
     * so the question renders before the request resolves, then updated in
     * place with `requestId`/`error`.
     */
    btwOverlay: Accessor<BtwOverlayState | null>;
    /** Close the /btw overlay (Esc / click-outside / explicit close). */
    closeBtw: () => void;
    /** The function also bound onto `SlashCommandContext.askSideQuestion`,
     *  exposed so callers and tests can drive `/btw` without the dispatcher. */
    askSideQuestion: (question: string) => Promise<{ requestId: string }>;
    /**
     * Run the controller refresh /login deferred because a turn was
     * streaming (see `SlashCommandContext.deferControllerRefreshUntilIdle`);
     * no-op if none is pending. Called on the turn-just-ended edge
     * (`turn-confirmation.ts`'s `trackTurnJustEnded`). Resolves true only if
     * a refresh ran and succeeded, so a caller holding an older
     * authFailureToPreserve snapshot knows it is stale (#2338).
     */
    flushPendingControllerRefresh: () => Promise<boolean>;
}

// Slash commands are data-driven via `frontend/app/view/agent/commands/`
// (docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md): adding one is
// a new file in `commands/global/` or `commands/providers/`, not an edit here.

export function useAgentCommands(opts: UseAgentCommandsOptions): UseAgentCommands {

    // Registry is rebuilt whenever the provider changes so
    // provider-scoped commands swap in/out. Global commands are
    // registered first and can't be shadowed (see registry.register).
    const registry = createMemo(() => buildRegistry(opts.provider()));

    // Pending-message expiry timers — cleared on unmount so a late
    // dispatch doesn't hit an unregistered slot and throw (#728, #742).
    const pendingExpiryTimers = new Set<ReturnType<typeof setTimeout>>();
    onCleanup(() => {
        for (const id of pendingExpiryTimers) clearTimeout(id);
        pendingExpiryTimers.clear();
    });

    // Messages typed while the agent is busy are HELD in `heldQueue` (the
    // "send now" panel) until `flushHeldMessages` delivers them at the next
    // tool-call boundary or ArrowUp recalls them (`recallLatestHeld`).
    // Holding is what makes recall a true un-send.
    // Images per message id, beside the queue rather than in each entry:
    // deliverToBackend looks them up by id for held and idle sends alike.
    // Removed once delivered or dropped.
    // SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.6.
    const attachmentsByMessage = new Map<string, AttachmentRef[]>();

    const heldQueue: Array<{
        id: string;
        text: string;
        authWasKnownBadAtQueueTime: boolean;
        /**
         * A live "auth" failure the caller captured before its TurnStart
         * cleared `state.failure` (idle-send hold only).
         * authWasKnownBadAtQueueTime can't carry it: a mid-turn 401/403
         * never sets canRetry/loginWaiting. Without it, an item whose
         * deferred refresh then FAILS is delivered to the bad credential.
         * Re-dispatched on rejection so the login banner returns (#2338).
         */
        authFailureToPreserve: PaneFailure | null;
        /**
         * True only for the idle-send hold, whose caller (handleSendMessage)
         * already dispatched an optimistic TurnStart; the busy-path hold
         * never did. On rejection this says whether to roll that TurnStart
         * back, or the pane stays "Working…" forever (#2338).
         */
        initiatedTurnOptimistically: boolean;
    }> = [];
    // Single-flight: the flush effect fires on every tool/phase change, and
    // two concurrent drains would interleave AgentInputCommands and reorder
    // sends (#1484). Holds the promise, not a flag, so a caller can await an
    // in-progress drain: sendMessage's idle path must let the drain that
    // flushPendingControllerRefresh starts finish before its own send (#2338).
    let inFlightHeldFlush: Promise<void> | null = null;
    onCleanup(() => {
        heldQueue.length = 0;
        inFlightHeldFlush = null;
    });

    // ── Inline picker state ───────────────────────────────────────────
    // The dispatcher calls `ctx.openPicker(spec)` for required enum/
    // dynamic args; this hook hands back a Promise that resolves when
    // the user picks (or rejects on Esc). The picker spec signal is
    // consumed by AgentPresentationView to render the picker overlay.
    const [pickerSpec, setPickerSpec] = createSignal<SlashPickerSpec | null>(null);
    let pickerResolver: ((value: string) => void) | null = null;
    let pickerRejecter: (() => void) | null = null;

    const openPicker = (spec: SlashPickerSpec): Promise<string> => {
        // If a previous picker is still open (shouldn't happen because
        // dispatch awaits), dismiss it cleanly so the new one wins.
        pickerRejecter?.();
        return new Promise<string>((resolve, reject) => {
            pickerResolver = resolve;
            pickerRejecter = reject;
            setPickerSpec(spec);
        });
    };

    const resolvePicker = (value: string): void => {
        const r = pickerResolver;
        pickerResolver = null;
        pickerRejecter = null;
        setPickerSpec(null);
        r?.(value);
    };

    const dismissPicker = (): void => {
        const r = pickerRejecter;
        pickerResolver = null;
        pickerRejecter = null;
        setPickerSpec(null);
        r?.();
    };

    // ── Help panel state ──────────────────────────────────────────────
    // /help calls ctx.openHelp(); the view reads helpVisible() and
    // mounts <SlashHelpPanel />. Stays open until the user dismisses.
    const [helpVisible, setHelpVisible] = createSignal(false);
    const openHelp = (): void => {
        setHelpVisible(true);
    };
    const closeHelp = (): void => {
        setHelpVisible(false);
    };

    // ── /btw overlay state ────────────────────────────────────────────
    // askSideQuestion (defined once below, shared by ctx and the return
    // value) both fires the request and opens/updates this signal; there is
    // no "open with nothing to ask" case, so unlike openPicker/openHelp it
    // isn't split into two context fields.
    const [btwOverlay, setBtwOverlay] = createSignal<BtwOverlayState | null>(null);
    // Monotonic per-hook counter — see BtwOverlayState.askId.
    let nextBtwAskId = 0;
    const closeBtw = (): void => {
        setBtwOverlay(null);
    };
    // Opens the overlay at once (question shown, requestId null), then
    // updates it when the request resolves. Rethrows after recording the
    // error so btw.ts's handler also returns it as a SlashResult error.
    // Updates match by askId, not question text: two asks can share text,
    // and a later ask's result must not land on an earlier one's (#3440).
    const askSideQuestion = async (question: string): Promise<{ requestId: string }> => {
        const askId = nextBtwAskId++;
        setBtwOverlay({ askId, question, requestId: null, error: null });
        try {
            const result = await (opts.askSideQuestion ?? (async () => ({ requestId: "" })))(question);
            setBtwOverlay((prev) =>
                prev && prev.askId === askId ? { ...prev, requestId: result.requestId } : prev,
            );
            return result;
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            setBtwOverlay((prev) => (prev && prev.askId === askId ? { ...prev, error: message } : prev));
            throw e;
        }
    };

    // Set by /login's finalizeLoginSuccess (login.ts) when it skips the
    // controller restart because a turn is streaming (a restart would kill
    // it). Persistent controllers live across turns, so dropping the restart
    // would leave the stale credential in place with every guard cleared
    // (#2338). Consumed by flushPendingControllerRefresh and
    // flushHeldMessages, fired off the same turn end by independent signals
    // (controllerstatus event vs. reactive turnPhase effect), either first.
    let controllerRefreshPendingUntilIdle = false;
    // The in-flight refresh, so a caller arriving after the other trigger
    // claimed the flag still awaits it; otherwise flushHeldMessages could
    // send while ControllerResyncCommand is still replacing the controller.
    let inFlightControllerRefresh: Promise<boolean> | null = null;
    // Bounded auto-retry for a failed deferred refresh. Re-arming the flag
    // alone isn't enough: the triggers that started the attempt have fired
    // and won't refire, so a held message could sit indefinitely. Bounded
    // because a persistent failure is better left to the user's next send
    // (which retries anyway) than to a timer hammering the resync RPC (#2338).
    const CONTROLLER_REFRESH_RETRY_DELAY_MS = 5000;
    const CONTROLLER_REFRESH_MAX_RETRIES = 3;
    let controllerRefreshRetriesRemaining = CONTROLLER_REFRESH_MAX_RETRIES;
    let refreshRetryTimer: ReturnType<typeof setTimeout> | null = null;
    // Checked before scheduling a retry: a refresh RPC still in flight at
    // dispose time can fail afterwards, and onCleanup (already run) would
    // never clear the timer it then creates.
    let isDisposed = false;
    onCleanup(() => {
        isDisposed = true;
        if (refreshRetryTimer) clearTimeout(refreshRetryTimer);
    });
    const deferControllerRefreshUntilIdle = (): void => {
        controllerRefreshPendingUntilIdle = true;
        // Fresh deferral: a full retry budget, not what an earlier one left.
        controllerRefreshRetriesRemaining = CONTROLLER_REFRESH_MAX_RETRIES;
    };
    // Resolves true only if it ran a refresh that succeeded. sendMessage's
    // idle path needs this to know its caller's authFailureToPreserve is
    // stale: TurnStart already cleared the live failure, so a re-read can't
    // tell "fixed by this refresh" from "cleared by TurnStart" (#2338).
    const flushPendingControllerRefresh = (): Promise<boolean> => {
        if (inFlightControllerRefresh) return inFlightControllerRefresh;
        if (!controllerRefreshPendingUntilIdle) return Promise.resolve(false);
        // Leave the flag pending unless the backend positively confirmed
        // idle, for every caller: a premature per-round session_end can make
        // turnPhase read idle while the backend still reports turn_active,
        // and every caller keys off turnPhase. Checked here once, centrally.
        // `!isBackendTurnConfirmedIdle()`, NOT `isBackendTurnActive()`: the
        // latter is also false when never confirmed (a pane mounted
        // mid-turn). This destructive action requires POSITIVE proof of
        // idle, not just absence of proof of activity (#2338).
        if (!opts.isBackendTurnConfirmedIdle()) return Promise.resolve(false);
        controllerRefreshPendingUntilIdle = false;
        inFlightControllerRefresh = (async () => {
            const refreshed = await opts.forceControllerRefresh();
            if (!refreshed) {
                // forceControllerRefresh failed without setting canRetry,
                // loginWaiting or failure, so re-arm: otherwise the next idle
                // send passes checkAuthGuard and reaches the stale
                // controller. Callers already treat "still pending" as hold
                // (#2338).
                controllerRefreshPendingUntilIdle = true;
                // Re-arming alone won't rerun it (see the retry constants
                // above): schedule a bounded retry so a transient failure
                // resolves itself. !isDisposed: see isDisposed.
                if (!isDisposed && controllerRefreshRetriesRemaining > 0) {
                    controllerRefreshRetriesRemaining -= 1;
                    if (refreshRetryTimer) clearTimeout(refreshRetryTimer);
                    refreshRetryTimer = setTimeout(() => {
                        refreshRetryTimer = null;
                        void flushPendingControllerRefresh();
                    }, CONTROLLER_REFRESH_RETRY_DELAY_MS);
                }
            }
            if (refreshed) {
                opts.notifyControllerHealthy();
                // Clear only an "auth" failure: FailureCleared clears any
                // code, and an unrelated one (rate_limited, ...) is still
                // real. See failure/useAuthHealth.ts's declareAuthHealthy.
                if (isAuthFailure(paneSnapshot(opts.blockId)?.failure)) {
                    opts.model.dispatchPane({ type: "FailureCleared" });
                }
                // Success proves the "auth" slot resolved, so held items'
                // older snapshots of it are stale; keeping them would reject
                // good messages and resurrect the "Not logged in" banner.
                // authWasKnownBadAtQueueTime is deliberately kept: it can
                // reflect a different, still-failing recovery that this
                // success says nothing about.
                for (const item of heldQueue) {
                    item.authFailureToPreserve = null;
                }
                // Drain messages held behind this refresh; nothing else is
                // guaranteed to: when turnPhase already reads idle (the case
                // deferral exists for) ReconcileTurnActive is a no-op, so the
                // reactive idle effect never refires. Fire-and-forget so
                // awaiting callers don't block on a full drain; a call from
                // inside flushHeldMessages hits its single-flight guard.
                if (heldQueue.length > 0) void flushHeldMessages();
            }
            return refreshed;
        })().finally(() => {
            inFlightControllerRefresh = null;
        });
        return inFlightControllerRefresh;
    };

    // The SlashCommandContext for sendMessage's dispatch and completions().
    // `wasAlreadyWorking` backs `isTurnActive` and MUST be the pre-TurnStart
    // snapshot — see SlashCommandContext.isTurnActive. Omitted
    // (completions/availableCommands never run a handler) means a live read.
    const buildCommandContext = (wasAlreadyWorking?: boolean): SlashCommandContext => ({
        blockId: opts.blockId,
        provider: opts.provider,
        block: opts.block,
        documentNodes: opts.documentNodes,
        log: opts.log,
        setAuthUrl: opts.setAuthUrl,
        notifyControllerHealthy: opts.notifyControllerHealthy,
        // Auth failures only — see flushPendingControllerRefresh above.
        clearAuthFailure: () => {
            if (isAuthFailure(paneSnapshot(opts.blockId)?.failure)) {
                opts.model.dispatchPane({ type: "FailureCleared" });
            }
        },
        forceControllerRefresh: opts.forceControllerRefresh,
        deferControllerRefreshUntilIdle,
        // false is frozen and true/undefined read live: see
        // SlashCommandContext.isTurnActive. On top of that (#2338):
        // - isBackendTurnActive() is ORed into both branches: a premature
        //   per-round session_end can demote turnPhase (and so the captured
        //   wasAlreadyWorking) while the backend still works, and backend
        //   events can't be corrupted by the optimistic TurnStart.
        // - The live branch also counts "not confirmed idle" as active, or
        //   a demoted turnPhase lets /login restart a working controller.
        //   Not on the frozen-false branch: a pane that never ran a turn is
        //   never confirmed idle, so /login would always defer.
        isTurnActive: () =>
            (wasAlreadyWorking === false
                ? false
                : workingFromPhase(paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" }) ||
                  !opts.isBackendTurnConfirmedIdle()) ||
            opts.isBackendTurnActive(),
        beginRecoveryFlow: opts.beginRecoveryFlow,
        endRecoveryFlow: opts.endRecoveryFlow,
        isCancelled: opts.isCancelled,
        resetCancelled: opts.resetCancelled,
        openPicker,
        openHelp,
        quickFork: opts.quickFork ?? (async () => false),
        askSideQuestion,
    });

    const completions = (prefix: string): SlashCommand[] => {
        return registry().completions(prefix, buildCommandContext());
    };

    const availableCommands = (): SlashCommand[] => {
        return registry().list(buildCommandContext());
    };

    const sendMessage = async (
        message: string,
        wasAlreadyWorking = false,
        authFailureToPreserve: PaneFailure | null = null,
        /** Images from the composer's tray, in order. A message with images is
         *  never a `!` or `/` command. */
        attachments: AttachmentRef[] = [],
    ): Promise<void> => {
        // Crash trace for "user pressed send": BlockErrorBoundary dumps the
        // trail on a renderer fault (frontend/log/render-trail.ts).
        trail("agent:send-message:enter", {
            blockId: opts.blockId,
            len: message.length,
        });
        // Intercept `!` and slash commands FIRST — client-side ones must not
        // touch the backend queue. Unknown `/foo` falls through to a turn.
        const trimmed = message.trim();

        // Restores the auth failure TurnStart cleared when a purely local
        // command (`!cmd`, or any slash command but a successful /login) ran
        // instead of a send. Otherwise it vanishes: the next send captures
        // null, canRetry/loginWaiting never saw it, and the guard lets that
        // send reach the bad credential (#2338).
        const restoreAuthFailureIfUnresolved = (clearedByCommand: boolean) => {
            // Only if the caller dispatched TurnStart: otherwise nothing
            // cleared the failure, and FailureObserved (which ends any
            // working turnPhase) would kill the real turn that is streaming.
            // The other re-dispatch sites roll back an optimistic turn first;
            // a busy pane has none to roll back.
            if (wasAlreadyWorking) return;
            if (authFailureToPreserve && !clearedByCommand) {
                opts.model.dispatchPane(
                    {
                        type: "FailureObserved",
                        failure: authFailureToPreserve.data,
                        at: Date.now(),
                        // Keep the original flag: the reducer defaults it to
                        // true, turning a pre-launch row into "Login Again"
                        // that resends an old message on an agent that never
                        // ran a turn. See PaneFailure.turnAttempted.
                        turnAttempted: authFailureToPreserve.turnAttempted,
                    },
                    "system",
                );
            }
        };

        const bangCommand = attachments.length > 0 ? null : parseBangCommand(trimmed);
        if (bangCommand !== null) {
            try {
                await dispatchBangCommand(bangCommand, opts.blockId, buildCommandContext(wasAlreadyWorking));
            } finally {
                // Undo handleSendMessage's TurnStart so the pane doesn't wait
                // out the 30s watchdog — only if it was idle at submit:
                // resetting a streaming turn would clobber its UI state.
                // TurnStartFailed, NOT TurnReset: no agent turn ran, so the
                // context meter and session totals stand (TurnReset wiped
                // them after every `!cmd`, `/help` or `/model`).
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
                }
                // Bang commands are shell execs — they never resolve auth.
                restoreAuthFailureIfUnresolved(false);
            }
            return;
        }

        if (attachments.length === 0 && trimmed.startsWith("/")) {
            let outcome;
            // Whether this command resolved the captured failure (only
            // /login, via ctx.clearAuthFailure()); restoring it then would
            // resurrect a stale banner.
            let authFailureClearedByCommand = false;
            const baseCtx = buildCommandContext(wasAlreadyWorking);
            const ctx: SlashCommandContext = {
                ...baseCtx,
                clearAuthFailure: () => {
                    authFailureClearedByCommand = true;
                    baseCtx.clearAuthFailure();
                },
            };
            try {
                outcome = await dispatchSlashCommand(trimmed, registry(), ctx);
            } catch {
                // dispatchSlashCommand threw — reset TurnStart so the pane
                // doesn't stay locked for the 30s watchdog window.
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
                }
                restoreAuthFailureIfUnresolved(authFailureClearedByCommand);
                return;
            }
            if (outcome.kind === "handled") {
                // No agent turn started; revert as in the bang path above.
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
                }
                restoreAuthFailureIfUnresolved(authFailureClearedByCommand);
                return;
            }
            // outcome.kind === "passthrough" — fall through to the real turn;
            // TurnStart stays active because an actual agent turn is about to happen.
        }

        // Init guard (#728, #742): TurnStart's Submitting is suppressed while
        // InitPending, but the message would still be queued and sent; if the
        // backend accepted before InitReady, the UI would show no turn while
        // the agent works.
        const ps = paneSnapshot(opts.blockId);
        if (ps?.initPhase.kind === "InitPending") {
            opts.log("send", "send blocked: history still loading", "warn");
            return;
        }

        // Shared by the pending entry and AgentInputCommand's `message_id`;
        // the backend echoes it in `agent-message-accepted`, and
        // `useAgentStream` then promotes the entry into a `user_message` node.
        const messageId = `user_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
        if (attachments.length > 0) attachmentsByMessage.set(messageId, attachments);

        // Into the pending zone, not `document`: acceptance promotes it.
        // Soft dispatch: a cascade can dispose the pane first (retro
        // 2026-05-23, agent-pane cascade → replaceChild quick-win).
        trail("agent:dispatch:PendingMessageQueued", { messageId });
        opts.model.dispatchPane(
            {
                type: "PendingMessageQueued",
                id: messageId,
                text: message,
                at: Date.now(),
                enqueuedWhileBusy: wasAlreadyWorking,
                ...(attachments.length > 0 ? { attachments } : {}),
            },
            "user",
        );
        trail("agent:dispatch:PendingMessageQueued:done", { messageId });

        // Defer the scroll-to-bottom by one animation frame so the
        // pending row has a chance to mount before the scroll math runs.
        if (opts.onSent) {
            requestAnimationFrame(() => opts.onSent?.());
        }

        if (wasAlreadyWorking) {
            // Busy: HOLD in the "send now" queue until flushHeldMessages
            // delivers it at the next tool-call boundary (or ArrowUp recalls
            // it). No expiry timer: it must persist until delivered.
            //
            // authWasKnownBadAtQueueTime is captured now, not re-checked at
            // flush: a turn can be active while auth is known bad. A flushed
            // item skips deliverToBackend's guard (initiatesTurn=false) so it
            // isn't dropped if auth turns bad later; that only holds if auth
            // was good at queue time, so flag the opposite case (#2338).
            // authFailureToPreserve is null: no TurnStart was dispatched, so
            // the live failure is still there for the flush's live check.
            const authWasKnownBadAtQueueTime = opts.canRetry() || opts.loginWaiting();
            if (authWasKnownBadAtQueueTime) {
                // Reject now rather than wait for a flush trigger that a
                // tool-less or stuck turn may never fire, leaving the message
                // stuck with no feedback.
                opts.log("auth", "message not sent — not logged in", "warn");
                attachmentsByMessage.delete(messageId);
                opts.model.dispatchPane({ type: "PendingMessageRejected", id: messageId });
                return;
            }
            heldQueue.push({ id: messageId, text: message, authWasKnownBadAtQueueTime, authFailureToPreserve: null, initiatedTurnOptimistically: false });
            // §2.3a (2026-09-17): if the turn is open only for backgrounded
            // work, flush now — a detached dev server may never produce
            // another tool call, and the dark indicator (working-indicator.ts)
            // promised an immediate answer. Pushed then flushed, not delivered
            // inline: the code below assumes an idle pane, while
            // flushHeldMessages is safe mid-turn. Awaited so sendMessage
            // resolves after the send.
            // Gated on the WHOLE `paneBusyForInput` predicate ("the indicator
            // is dark"): `turnHeldOnlyByBackgroundWork` alone ignores
            // compacting/reconnecting and delivered mid-compaction (#3340).
            // Do not narrow it to a subset.
            const live = paneSnapshot(opts.blockId);
            if (
                !paneBusyForInput(
                    busyInputFromState(live, opts.documentNodes(), opts.showingLaunchActivity?.() ?? false),
                )
            ) {
                await flushHeldMessages();
            }
            return;
        }

        // session_end -> TurnEnd (useTurnLifecycle.ts's finalizeTurn) makes
        // turnPhase idle before the controllerstatus event that runs the
        // deferred refresh arrives. Run it here so a send in that gap can't
        // reach the stale controller before ControllerResyncCommand (#2338).
        // No-ops when nothing is pending.
        const refreshedByDeferral = await flushPendingControllerRefresh();

        // Still pending after that call means a refresh is deferred but the
        // backend hasn't confirmed idle (the flag clears only once a refresh
        // commits). Hold rather than deliver to a possibly stale or busy
        // controller; flushHeldMessages drains it once idle is confirmed.
        if (controllerRefreshPendingUntilIdle) {
            // TurnStart already cleared the live failure for this idle send,
            // so authFailureToPreserve is the only record that auth was bad;
            // carry it so a later failed refresh can't deliver this item.
            heldQueue.push({ id: messageId, text: message, authWasKnownBadAtQueueTime: opts.canRetry() || opts.loginWaiting(), authFailureToPreserve, initiatedTurnOptimistically: true });
            return;
        }

        // A successful deferred refresh started a background drain of older
        // held messages; await it so they go out first (FIFO) instead of
        // racing this send's AgentInputCommand.
        if (refreshedByDeferral) {
            await flushHeldMessages();
        }

        // Idle send: deliver immediately and arm the lost-delivery safety timer.
        // Drop the caller's authFailureToPreserve if the deferred refresh just
        // succeeded: it predates the fix and would reject the message and
        // resurrect the "Not logged in" banner.
        await deliverToBackend(
            message,
            messageId,
            /* armExpiry */ true,
            /* initiatesTurn */ true,
            refreshedByDeferral ? null : authFailureToPreserve,
        );
    };

    /**
     * Deliver a single message to the backend: apply runtime args (permission
     * mode / model / effort) for this turn, fire `AgentInputCommand`, and —
     * for immediate (idle) sends — arm the lost-delivery expiry. The backend
     * echoes `agent-message-accepted` (keyed on `messageId`), which promotes
     * the pending entry into the conversation.
     */
    const deliverToBackend = async (
        message: string,
        messageId: string,
        armExpiry: boolean,
        /** True when this message's optimistic TurnStart started the turn
         *  (an idle send, or a flushed idle-send hold). False for a message
         *  held while a real turn ran: its failure must not end that turn. */
        initiatesTurn: boolean,
        /** Caller's pre-TurnStart capture of an "auth" failure (see
         *  sendMessage). Always null for a held-message flush. */
        authFailureToPreserve: PaneFailure | null,
    ): Promise<void> => {
        // Fast-fail when the pane already knows auth is bad, instead of a
        // doomed "Working…" round-trip: nothing between here and the CLI
        // spawn re-verifies auth. Rejects like a failed AgentInputCommand
        // (docs/retro/retro-send-while-unauthenticated-2026-07-28.md).
        // Signals (#2338):
        // - canRetry(): the pane knows it is logged out.
        // - loginWaiting(): a login attempt is unresolved (canRetry clears
        //   on click, before the up-to-5-minute OAuth poll confirms).
        // - authFailureToPreserve: a mid-turn 401/403, which neither signal
        //   sees; TurnStart cleared it, so the caller captured it.
        // Only when initiatesTurn: a message held during a real turn must not
        // be silently dropped if auth turns bad mid-turn (held messages wait
        // until delivered); a real failure is handled by the catch below.
        // loginWaiting() is never bypassed, even for a recovery flow's
        // auto-retry: a sibling flow still running force-restarts the
        // controller when it finishes, killing the retried turn. The blocked
        // retry fires again from the last flow's onRecovered.
        // Returns false after dispatching every rejection side effect. Run
        // twice: the SetMetaCommand round-trip between is an async gap a
        // recovery flow or a mid-turn failure can land in.
        const checkAuthGuard = (): boolean => {
            const loginStillWaiting = opts.loginWaiting();
            // Also a live read, for an auth failure that lands during the
            // SetMeta gap. TurnStart already fired, so it can't double-count
            // the captured one, and canRetry/loginWaiting don't see a 401/403.
            const liveAuthFailure = isAuthFailure(paneSnapshot(opts.blockId)?.failure);
            if (!(initiatesTurn && (opts.canRetry() || loginStillWaiting || authFailureToPreserve || liveAuthFailure))) {
                return true;
            }
            opts.log("auth", "message not sent — not logged in", "warn");
            // Only with no failure row on screen. The "Log in" bar was removed
            // (PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md) and its
            // replacement row is dismissible, so there is no button to point
            // at: name the recovery that always works.
            if (!authFailureToPreserve && !liveAuthFailure) {
                opts.setAuthNotice(
                    loginStillWaiting
                        ? "Not logged in yet — wait for the login attempt to finish, then try again."
                        : "Not logged in — run /login to sign in, then send again.",
                );
            }
            attachmentsByMessage.delete(messageId);
            opts.model.dispatchPane({
                type: "PendingMessageRejected",
                id: messageId,
            });
            if (initiatesTurn) {
                opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
            }
            if (authFailureToPreserve) {
                // Re-dispatch the failure TurnStart cleared: its banner's
                // login actions are the pane's recovery path, which a generic
                // notice lacks (#2338, #2951). canRetry() can also be true
                // here: this may be the synthetic pre-launch failure
                // (turnAttempted=false). After TurnStartFailed, so
                // turnPhase is already Idle; otherwise the reducer reads
                // Submitting as a turn ending and flashes Done.
                opts.model.dispatchPane(
                    {
                        type: "FailureObserved",
                        failure: authFailureToPreserve.data,
                        at: Date.now(),
                        turnAttempted: authFailureToPreserve.turnAttempted, // see PaneFailure.turnAttempted
                    },
                    "system",
                );
            }
            return false;
        };
        if (!checkAuthGuard()) return;

        // Apply runtime args (permission mode, model, effort) before this turn.
        const prov = opts.provider();
        if (prov) {
            const runtimeConfig = getRuntimeConfig(opts.block()?.meta);
            // A container agent runs one `docker exec` per turn and must never
            // get the persistent controller's args (`--input-format
            // stream-json` makes the CLI parse the startup markdown as JSON
            // and die). This runs before every send, so it matters most (#2867).
            const meta = opts.block()?.meta;
            const agentMode = meta?.["agentMode"] as string | undefined;
            // Reapply the agent's provider_flags: this rebuild starts from the
            // provider catalog and would drop them on the first send (#2872).
            // Not `--fork-session`: one-shot, it would fork every turn.
            const updatedArgs = buildPaneArgs(prov, agentMode, runtimeConfig, meta?.[PROVIDER_FLAGS_META_KEY]);
            try {
                await setBlockMeta(opts.blockId, { "cmd:args": updatedArgs });
            } catch (err) {
                opts.log("error", `Failed to update runtime args: ${err}`, "error");
            }
        }

        // Re-check after the SetMetaCommand gap — see checkAuthGuard.
        if (!checkAuthGuard()) return;

        // Awaited so the flush loop keeps submission order (#1484). The
        // cmd:args round-trip is done, so the 30s expiry below measures only
        // backend acceptance (#752).
        try {
            const attachments = attachmentsByMessage.get(messageId);
            await RpcApi.AgentInputCommand(TabRpcClient, {
                blockid: opts.blockId,
                message,
                message_id: messageId,
                ...(attachments?.length ? { attachments } : {}),
            });
            attachmentsByMessage.delete(messageId);
        } catch (err: any) {
            attachmentsByMessage.delete(messageId);
            opts.log("error", err?.message ?? String(err), "error");
            // RPC outright failed — remove the pending entry so the user
            // doesn't see a ghost row for a message the backend never received.
            opts.model.dispatchPane({
                type: "PendingMessageRejected",
                id: messageId,
            });
            // Undo the optimistic TurnStart if this send started the turn: a
            // synchronous failure (no controller after a backend restart, the
            // identity spawn gate, a network rejection) otherwise leaves
            // "Working…" forever, since PendingMessageRejected skips turnPhase.
            // TurnStartFailed, NOT TurnReset: TurnReset also wipes
            // sessionStats/sessionTotals/context, which a transient
            // send failure must keep (#2318).
            if (initiatesTurn) {
                opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
            }
            return;
        }

        if (!armExpiry) return;
        // Pending acceptance timeout (#728), idle sends only: no
        // `agent-message-accepted` within PENDING_TIMEOUT_MS means the delivery
        // was lost. Held messages get NO expiry; they wait until flushed.
        const expiryId = setTimeout(() => {
            pendingExpiryTimers.delete(expiryId);
            opts.model.dispatchPane({
                type: "PendingMessageExpired",
                id: messageId,
            });
        }, PENDING_TIMEOUT_MS);
        pendingExpiryTimers.add(expiryId);
    };

    /**
     * Deliver every held ("send now") message, oldest first. Called at the next
     * tool-call boundary / turn end. No expiry — these are being delivered now,
     * and a failed RPC removes the pending entry via the catch in
     * `deliverToBackend`.
     */
    const flushHeldMessages = (): Promise<void> => {
        // Single-flight — see inFlightHeldFlush. The loop re-checks the
        // queue, so messages queued mid-drain still go out in order.
        if (inFlightHeldFlush) return inFlightHeldFlush;
        inFlightHeldFlush = (async () => {
            // Only when turnPhase is idle: this also runs at mid-turn
            // tool-call boundaries (useHeldMessageDelivery) and from
            // Esc-to-steer (handleEscapeOnEmptyComposer), where a restart
            // would kill the turn. When idle, a deferred restart (pending, or
            // in flight from the other turn-end trigger) must finish before
            // any AgentInputCommand, or a held message hits the stale or
            // restarting controller (#2338).
            if (!workingFromPhase(paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" })) {
                await flushPendingControllerRefresh();
                // Still pending means the backend hasn't confirmed idle, so
                // the controller may still be stale: don't drain. Nothing is
                // lost — a successful refresh re-drains this queue. Only in
                // this branch: mid-turn, a pending flag is unrelated and
                // draining at the tool-call boundary is intended.
                if (controllerRefreshPendingUntilIdle) {
                    return;
                }
            }

            // Drain one at a time, awaiting each delivery (incl. its cmd:args
            // round-trip) so messages reach the CLI's stdin in submission order.
            // shift() (not a snapshot) so items queued mid-flush are included.
            while (heldQueue.length > 0) {
                const item = heldQueue.shift()!;
                // Reject a held item if any of these holds (#2338):
                // (1) authWasKnownBadAtQueueTime — always, never re-derived
                //     from live canRetry/loginWaiting: a FAILED recovery
                //     clears loginWaiting without setting canRetry, so both
                //     read clean while nothing is fixed. A rare lost message
                //     is the price of never sending on an unproven credential.
                // (2) a live "auth" failure — the turn hit a 401/403 after
                //     queueing, which the frozen flag misses and, for a
                //     busy-path item (initiatesTurn=false), deliverToBackend's
                //     guard skips.
                // (3) authFailureToPreserve — captured before TurnStart
                //     cleared it; else a failed deferred refresh delivers it.
                // turnPhase is left alone (a real turn is running) except to
                // roll back an idle-send hold's TurnStart, below.
                const liveAuthFailure = isAuthFailure(paneSnapshot(opts.blockId)?.failure);
                if (item.authWasKnownBadAtQueueTime || item.authFailureToPreserve || liveAuthFailure) {
                    opts.log("auth", "held message not sent — not logged in", "warn");
                    // Roll back the idle-send hold's optimistic TurnStart, before
                    // FailureObserved (as in checkAuthGuard), or the pane stays
                    // "Working…": deliverToBackend's TurnStartFailed never runs.
                    if (item.initiatedTurnOptimistically) {
                        opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
                    }
                    // Restore the login banner TurnStart cleared, when this
                    // item's snapshot caused the rejection and no live failure
                    // covers it (as in restoreAuthFailureIfUnresolved).
                    if (item.authFailureToPreserve && !liveAuthFailure) {
                        opts.model.dispatchPane(
                        {
                            type: "FailureObserved",
                            failure: item.authFailureToPreserve.data,
                            at: Date.now(),
                            turnAttempted: item.authFailureToPreserve.turnAttempted, // see PaneFailure.turnAttempted
                        },
                        "system",
                    );
                    }
                    attachmentsByMessage.delete(item.id);
                    opts.model.dispatchPane({ type: "PendingMessageRejected", id: item.id });
                    continue;
                }
                // initiatesTurn from the item, not false: if an idle-send
                // hold's RPC fails, its optimistic TurnStart must be rolled
                // back, and with no expiry timer nothing else would (#2338).
                // Mark the entry `flushing` only now, when delivery starts
                // (not at TurnEnd, possibly long before if the branch above
                // bailed), so PendingMessagesPanel's "Sending…" is true
                // (SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md
                // Phase 1, #2970).
                opts.model.dispatchPane({ type: "PendingMessageFlushStarted", id: item.id });
                await deliverToBackend(item.text, item.id, /* armExpiry */ false, /* initiatesTurn */ item.initiatedTurnOptimistically, /* authFailureToPreserve */ null);
            }
        })().finally(() => {
            inFlightHeldFlush = null;
        });
        return inFlightHeldFlush;
    };

    /**
     * Pop the most-recently held message off the queue, remove its pending
     * entry, and return its text so the composer can restore it (ArrowUp
     * un-queue). Null if nothing is held. The message was never sent.
     */
    const recallLatestHeld = (): { text: string; attachments?: AttachmentRef[] } | null => {
        const item = heldQueue.pop();
        if (!item) return null;
        // Roll back an idle-send hold's optimistic TurnStart and restore the
        // login banner its snapshot carries, as in flushHeldMessages's
        // rejection path; otherwise recall leaves "Working…" forever and
        // drops the banner (#2338).
        if (item.initiatedTurnOptimistically) {
            opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
        }
        if (item.authFailureToPreserve && paneSnapshot(opts.blockId)?.failure?.data.code !== "auth") {
            opts.model.dispatchPane(
                        {
                            type: "FailureObserved",
                            failure: item.authFailureToPreserve.data,
                            at: Date.now(),
                            turnAttempted: item.authFailureToPreserve.turnAttempted, // see PaneFailure.turnAttempted
                        },
                        "system",
                    );
        }
        opts.model.dispatchPane({ type: "PendingMessageRejected", id: item.id });
        const attachments = attachmentsByMessage.get(item.id);
        attachmentsByMessage.delete(item.id);
        return attachments?.length ? { text: item.text, attachments } : { text: item.text };
    };

    const hasHeldMessages = (): boolean => heldQueue.length > 0;

    // Delegate to the model so the pane-frame header button and any other
    // call sites go through a single implementation.
    const back = async (): Promise<void> => {
        await opts.backToPicker();
    };

    const stopAgent = (): void => {
        // Only stop a working turn: Esc on an idle pane should be a quiet
        // no-op, not a SIGINT that lands nowhere and logs "stop failed".
        const phase = paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" as const };
        if (!workingFromPhase(phase)) return;

        // turnPhase → Interrupting shows "Stopping…" at once. `useAgentStream`
        // finalizes: on `session_end` it dispatches TurnEnd (Done.stopped) and
        // appends "⏹ Interrupted", with a fallback timer in case the killed
        // CLI never emits it. Soft dispatch: see PendingMessageQueued above.
        opts.model.dispatchPane({ type: "RequestStop", at: Date.now() }, "user");
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: opts.blockId,
            signame: "SIGINT",
        }).catch((err) => {
            opts.log("warn", `stop failed: ${err?.message ?? String(err)}`, "warn");
            opts.model.dispatchPane({ type: "StopFailed" });
        });
    };

    return {
        sendMessage,
        flushHeldMessages,
        recallLatestHeld,
        hasHeldMessages,
        back,
        stopAgent,
        pickerSpec,
        resolvePicker,
        dismissPicker,
        completions,
        helpVisible,
        closeHelp,
        availableCommands,
        flushPendingControllerRefresh,
        btwOverlay,
        closeBtw,
        askSideQuestion,
    };
}

/**
 * Run a `!`-prefixed composer message as a shell command in the agent's
 * working directory via the `shellexec` RPC; never reaches the agent queue.
 * stdout/stderr go to the shell drawer as system-tagged log lines (see
 * agent-view.tsx's `log`). `blockId` is passed explicitly for the RPC
 * payload; `ctx` supplies only the log sink and block metadata.
 */
async function dispatchBangCommand(
    command: string,
    blockId: string,
    ctx: SlashCommandContext,
): Promise<void> {
    if (!command) {
        ctx.log("system", "!: command required", "warn");
        return;
    }
    const workingDir = (ctx.block()?.meta?.["cmd:cwd"] as string | undefined) ?? "";
    ctx.log("system", `$ ${command}`);
    try {
        const result = await RpcApi.ShellExecCommand(
            TabRpcClient,
            { blockid: blockId, command, working_dir: workingDir },
            // Long timeout: shell commands like `npm test` or `cargo build` can
            // take minutes. Default 5s RPC timeout would return EC-TIME immediately.
            { timeout: 300_000 },
        );
        if (result.stdout) ctx.log("system", result.stdout.trimEnd());
        if (result.stderr) ctx.log("system", result.stderr.trimEnd(), "warn");
        if (result.exit_code !== 0) {
            ctx.log("system", `exit ${result.exit_code}`, "warn");
        }
    } catch (e) {
        ctx.log("system", `!: ${(e as Error).message ?? String(e)}`, "warn");
    }
}
