// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentCommands — owns the top-level user-driven handlers for the
 * agent pane: sending messages (including slash-command intercepts)
 * and returning to the agent picker.
 *
 * Step 12 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Extracted from agent-view.tsx so AgentPresentationView stays focused
 * on composition + JSX instead of owning several dozen lines of
 * RPC plumbing.
 *
 * Slash commands intercepted:
 *   - `/login` — runs a GUI OAuth flow via the host API, captures the
 *                returned URL, and pushes it into `setAuthUrl` so the
 *                auth box appears above the composer.
 *   - `/clear` — frontend-only document reset.
 *
 * All other messages pass through to the backend via
 * `RpcApi.AgentInputCommand`, with `cmd:args` updated first so runtime
 * flags (permission mode, model, effort) take effect on this turn.
 */

import { type Accessor, createMemo, createSignal, onCleanup } from "solid-js";
import { trail } from "@/log/render-trail";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { buildRuntimeArgs, getRuntimeConfig } from "../buildRuntimeArgs";
import { dispatchSlashCommand } from "../commands/dispatch";
import { buildRegistry } from "../commands/registry";
import type { SlashCommand, SlashCommandContext, SlashPickerSpec } from "../commands/types";
import type { ProviderDefinition } from "../providers";
import type { SignalPair } from "../state";
import type { DocumentNode } from "../types";
import type { LogFn } from "./useAgentControllerStatus";

/**
 * How long a pending message can sit unacknowledged before the reducer
 * gives up and removes it. 30s covers normal backend turnaround
 * (typically <2s on local sockets) with margin for transient hiccups.
 * Issue #728 gap 2.
 */
const PENDING_TIMEOUT_MS = 30_000;

export interface UseAgentCommandsOptions {
    blockId: string;
    /**
     * Per-pane model handle returned by `registerPane`. Threaded in so
     * the hook can dispatch via `model.dispatchPane` — default-safe
     * against post-unmount races. PR-4 of the cascade follow-up
     * sequence. See `agent-pane-model.ts`.
     */
    model: AgentPaneModel;
    block: Accessor<{ meta?: Record<string, any> } | undefined>;
    provider: Accessor<ProviderDefinition | undefined>;
    documentAtom: SignalPair<DocumentNode[]>;
    log: LogFn;
    setAuthUrl: (url: string | null) => void;
    /**
     * True when the pane is ALREADY showing the mount-time "Log in" bar
     * (`useAgentControllerStatus`'s `canRetry`) — no turn was ever
     * attempted, or a recovery attempt just failed. `deliverToBackend`
     * checks this immediately before `AgentInputCommand` so a message
     * sent from a pane the UI already knows is logged out fails fast
     * instead of spawning a doomed CLI process that only reports "Not
     * logged in" after a real network round-trip. See
     * docs/retro/retro-send-while-unauthenticated-2026-07-28.md.
     */
    canRetry: Accessor<boolean>;
    /**
     * True while `relogin()`/`loginViaTerminal()`/`useGlobalLogin()`'s own
     * recovery attempt is in flight. `canRetry` is cleared the INSTANT the
     * mount-time "Log in" button is clicked (relogin()'s own first line),
     * well before that attempt actually succeeds or fails, so `canRetry`
     * alone leaves this whole window unguarded: a message sent mid-attempt
     * would reach `AgentInputCommand` on a credential the pane already has
     * reason to distrust, reproducing the exact doomed spawn this fast-fail
     * exists to prevent. Codex P1 on PR #2338; useGlobalLogin() was missing
     * this entirely (never touched `loginWaiting` at all) until reagent P1
     * caught it on re-review.
     */
    loginWaiting: Accessor<boolean>;
    /** Same surface `useAgentControllerStatus`'s own recovery-failure
     *  notices use (the error box above the composer) — reused here so
     *  the known-unauthenticated fast-fail above looks consistent with
     *  every other auth-recovery message this pane can show. */
    setAuthNotice: (notice: string | null) => void;
    /**
     * `useAgentControllerStatus.notifyControllerHealthy` — threaded into
     * the slash-command context so /login's handler can clear a stale
     * `canRetry` on success. /login never goes through `relogin()` (the
     * only other place that manages `canRetry`), so without this a pane
     * that typed /login directly instead of clicking "Log in" would have
     * every later message fast-failed forever. Codex P1 on PR #2338.
     */
    notifyControllerHealthy: () => void;
    /**
     * `useAgentControllerStatus.forceControllerRefresh` — threaded into the
     * slash-command context so /login's handler can restart an already-
     * running persistent controller onto its newly-refreshed credential,
     * matching relogin()/useGlobalLogin()/loginViaTerminal(). Without this,
     * a successful /login on a pane whose controller was already alive
     * left it on the stale credential — the next message bypasses every
     * guard in this PR (canRetry/loginWaiting/authFailureToPreserve are
     * all correctly cleared by then) and still reaches the stale process.
     * Codex P1 on PR #2338 (seventh re-review).
     */
    forceControllerRefresh: () => Promise<void>;
    /**
     * The model-level backToPicker action. The hook delegates to this
     * rather than owning a duplicate implementation — the pane-frame
     * header button also calls it, so the logic needs to live in one
     * place (AgentViewModel). See SPEC_AGENT_PANE_FOLLOWUPS item #8.
     */
    backToPicker: () => Promise<void>;
    /**
     * Called on the next animation frame after a user_message is
     * appended to the document via `sendMessage`. AgentPresentationView
     * wires this to the AgentDocumentView's `scrollToBottomFn` so the
     * user's own message is guaranteed visible when they press Enter.
     * Without this, the auto-scroll effect may be skipped if `autoScroll`
     * was flipped off during the composer's own growth.
     * See SPEC_AGENT_PANE_FOLLOWUPS item #1.
     */
    onSent?: () => void;
    /**
     * Queue of messages sent to the backend but not yet accepted.
     * `sendMessage` appends here (instead of directly to the document)
     * and `useAgentStream` removes entries on `agent-message-accepted`,
     * promoting them into the document at that moment.
     */
    pendingMessagesAtom?: SignalPair<import("../state").PendingMessage[]>;
}

export interface UseAgentCommands {
    /** Send a user message. Slash commands are intercepted via the registry.
     *  `wasAlreadyWorking` should be true when a turn was in-flight before
     *  the caller dispatched TurnStart — drives PendingMessage.enqueuedWhileBusy.
     *  `authFailureToPreserve` should be the caller's own pre-TurnStart read
     *  of `state.failure` when it was "auth"-classified (null otherwise) —
     *  TurnStart unconditionally clears that failure, so `deliverToBackend`
     *  can't re-derive it live; the caller must capture it before
     *  dispatching TurnStart. When present, the fast-fail guard rejects the
     *  send AND re-dispatches this same failure so the banner (and its
     *  "Login Again"/"Use existing login" actions) reappears instead of
     *  vanishing with no path back. Codex P1 on PR #2338 (third re-review).
     *  `trustedAfterRecovery` should be true ONLY for the auto-retry
     *  `retryLastTurn` fires from a recovery flow's own `onRecovered`
     *  callback — bypasses the loginWaiting() check specifically, since
     *  loginWaiting is a shared counter across all three recovery flows and
     *  can still read true from a DIFFERENT, unrelated flow overlapping
     *  with the one that just confirmed success. Codex P2 on PR #2338
     *  (fifth re-review). */
    sendMessage: (
        message: string,
        wasAlreadyWorking?: boolean,
        authFailureToPreserve?: AgentFailure | null,
        trustedAfterRecovery?: boolean,
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
    recallLatestHeld: () => { text: string } | null;
    /** True when there are queued-while-busy messages awaiting delivery. */
    hasHeldMessages: () => boolean;
    /** Return to the agent picker by clearing the agent-identity meta keys. */
    back: () => Promise<void>;
    /**
     * Send SIGINT to the currently running agent CLI process. Invoked
     * from the composer's Esc handler when the textarea is empty —
     * equivalent to Ctrl+C in a terminal. Silently no-ops if the
     * controller rejects the signal (e.g. no process running).
     * See SPEC_AGENT_PANE_FOLLOWUPS item #9.
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
}

// Runtime-config + auth slash commands (/model /effort /permission-mode
// /bypass /plan /runtime /login /clear) are now data-driven via
// `frontend/app/view/agent/commands/`. See
// specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md for the design.
// sendMessage below dispatches through the registry; adding a new
// command is one file create in `commands/global/` or
// `commands/providers/`, not an edit here.

export function useAgentCommands(opts: UseAgentCommandsOptions): UseAgentCommands {

    // Registry is rebuilt whenever the provider changes so
    // provider-scoped commands swap in/out. Global commands are
    // registered first and can't be shadowed (see registry.register).
    const registry = createMemo(() => buildRegistry(opts.provider()));

    // Pending-message expiry timers — cleared on pane unmount so the
    // delayed dispatch doesn't hit an unregistered slot and throw.
    // Issue #728 gap 2 / PR #742 ReAgent P1.
    const pendingExpiryTimers = new Set<ReturnType<typeof setTimeout>>();
    onCleanup(() => {
        for (const id of pendingExpiryTimers) clearTimeout(id);
        pendingExpiryTimers.clear();
    });

    // Messages typed while the agent was busy are HELD here (not sent yet),
    // shown in the "send now" panel. They are delivered by `flushHeldMessages`
    // at the next tool-call boundary, or recalled un-sent via `recallLatestHeld`
    // (ArrowUp). Holding — rather than sending immediately — is what makes the
    // recall a true un-send and lets the message land at a clean boundary.
    const heldQueue: Array<{ id: string; text: string; authWasKnownBadAtQueueTime: boolean }> = [];
    // Re-entrancy guard: only ONE flush drains the queue at a time. The flush
    // effect fires fire-and-forget on every tool/phase change, so without this
    // a second boundary could start a concurrent flush whose AgentInputCommand
    // interleaves with the first's — reordering sends. ReAgent P2 on PR #1484.
    let flushing = false;
    onCleanup(() => {
        heldQueue.length = 0;
        flushing = false;
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

    // Build the SlashCommandContext bundle. Used by sendMessage's
    // dispatch and by completions(); both need the same view of the
    // pane's reactive state.
    const buildCommandContext = (): SlashCommandContext => ({
        blockId: opts.blockId,
        provider: opts.provider,
        block: opts.block,
        documentAtom: opts.documentAtom,
        log: opts.log,
        setAuthUrl: opts.setAuthUrl,
        notifyControllerHealthy: opts.notifyControllerHealthy,
        clearAuthFailure: () => opts.model.dispatchPane({ type: "FailureCleared" }),
        forceControllerRefresh: opts.forceControllerRefresh,
        openPicker,
        openHelp,
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
        authFailureToPreserve: AgentFailure | null = null,
        trustedAfterRecovery = false,
    ): Promise<void> => {
        // Crash trace: this is the entry point for "user pressed send."
        // The boundary dumps this trail when a renderer fault catches —
        // see frontend/log/render-trail.ts + BlockErrorBoundary.
        trail("agent:send-message:enter", {
            blockId: opts.blockId,
            len: message.length,
        });
        // Intercept slash commands FIRST — some (/clear, /login) are
        // handled client-side and must not touch the backend queue at
        // all. Unknown `/foo` falls through to a real turn.
        const trimmed = message.trim();

        // Bang commands: `!cmd` runs a shell command in the agent's working
        // directory and surfaces output in the launch log. Never falls through
        // to the backend agent queue.
        // TurnStart was already dispatched by handleSendMessage (agent-view.tsx)
        // before this function was called. Reset it immediately so the pane does
        // not stay in Submitting state for the full 30s watchdog window — no
        // agent turn was initiated, only a sidecar shell exec.
        if (trimmed.startsWith("!")) {
            try {
                await dispatchBangCommand(trimmed.slice(1).trim(), opts.blockId, buildCommandContext());
            } finally {
                // TurnStart was dispatched by handleSendMessage before sendMessage
                // was called. Reset it so the pane returns to Idle instead of waiting
                // for the 30s watchdog — but only if the pane was idle when !cmd was
                // submitted. If wasAlreadyWorking is true, a real agent turn was
                // already streaming; resetting to Idle here would clobber its UI state
                // until the next backend event arrives.
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnReset" }, "system");
                }
            }
            return;
        }

        if (trimmed.startsWith("/")) {
            let outcome;
            try {
                outcome = await dispatchSlashCommand(trimmed, registry(), buildCommandContext());
            } catch {
                // dispatchSlashCommand threw — reset TurnStart so the pane
                // doesn't stay locked for the 30s watchdog window.
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnReset" }, "system");
                }
                return;
            }
            if (outcome.kind === "handled") {
                // TurnStart was dispatched by handleSendMessage before sendMessage
                // was called. Reset it so the pane returns to Idle — no real agent
                // turn was initiated, only a client-side command. Mirrors the bang
                // command's finally-TurnReset above.
                if (!wasAlreadyWorking) {
                    opts.model.dispatchPane({ type: "TurnReset" }, "system");
                }
                return;
            }
            // outcome.kind === "passthrough" — fall through to the real turn;
            // TurnStart stays active because an actual agent turn is about to happen.
        }

        // Init guard (issue #728 gap 1, codex P2 on PR #742). The
        // reducer's TurnStart handler already suppresses the
        // Submitting transition while initPhase.kind === "InitPending",
        // but that only stops the local UI state — without this
        // check, the message still gets queued into pending AND sent
        // over AgentInputCommand. If the backend accepts before
        // InitReady fires, the accepted-event TurnStart is also
        // suppressed, leaving the UI showing no active turn while the
        // agent IS processing. Bail early here so neither happens.
        const ps = paneSnapshot(opts.blockId);
        if (ps?.initPhase.kind === "InitPending") {
            opts.log("send", "send blocked: history still loading", "warn");
            return;
        }

        // Stable id shared between the pending entry and the backend's
        // `message_id` field on `AgentInputCommand`. The backend echoes
        // it via `agent-message-accepted` when it picks up this config,
        // and `useAgentStream` uses that to promote the pending entry
        // into a real `user_message` document node.
        const messageId = `user_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;

        // Append to the pending zone. No direct write to `document` —
        // the acceptance event promotes it. This is the architecture
        // from AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md (two lists,
        // migration on accept).
        // Soft variant — cascade-during-dispatch could dispose the pane
        // before this fires; retro 2026-05-23 (agent-pane cascade →
        // replaceChild quick-win).
        trail("agent:dispatch:PendingMessageQueued", { messageId });
        opts.model.dispatchPane(
            {
                type: "PendingMessageQueued",
                id: messageId,
                text: message,
                at: Date.now(),
                enqueuedWhileBusy: wasAlreadyWorking,
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
            // Agent is busy: HOLD the message in the "send now" queue instead of
            // sending it now. `flushHeldMessages` delivers it at the next
            // tool-call boundary (so the agent finishes its current step first),
            // and ArrowUp can recall it un-sent before then. No expiry timer —
            // the message must persist until it is actually delivered, not drop
            // off after 30s (the bug this fixes).
            //
            // authWasKnownBadAtQueueTime: captured HERE, not re-checked live
            // at flush time. This pane's own "wasAlreadyWorking" turnPhase and
            // canRetry()/loginWaiting() are tracked independently — nothing
            // enforces "if a turn is active, auth must be fine" — so a
            // controller reporting an active turn while the mount-time auth
            // check has ALSO shown "Log in" (or a recovery attempt is still
            // resolving) is a real, reachable combination, not a contradiction.
            // deliverToBackend's guard is deliberately gated on initiatesTurn
            // (false for every flushed item) so a message that becomes
            // enqueued-while-legitimately-busy doesn't get retroactively
            // dropped just because canRetry()/loginWaiting() flip true AFTER
            // it was queued (Codex P1, earlier re-review) — but that reasoning
            // only holds when auth was GOOD at queue time. Capture the
            // opposite case here so flushHeldMessages can reject exactly
            // those items instead of blindly trusting initiatesTurn=false
            // for all of them. Codex P2 on PR #2338 (sixth re-review).
            heldQueue.push({ id: messageId, text: message, authWasKnownBadAtQueueTime: opts.canRetry() || opts.loginWaiting() });
            return;
        }

        // Idle send: deliver immediately and arm the lost-delivery safety timer.
        await deliverToBackend(message, messageId, /* armExpiry */ true, /* initiatesTurn */ true, authFailureToPreserve, trustedAfterRecovery);
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
        /** True only for the idle-send path (this message is what put the
         *  pane into Submitting/Streaming via handleSendMessage's optimistic
         *  `TurnStart`, before this RPC call ever ran). False for a held
         *  message flushed mid-turn — a real turn is already genuinely in
         *  flight there, independent of whether THIS specific flushed
         *  message's delivery succeeds, so its failure must not cut that
         *  turn short. */
        initiatesTurn: boolean,
        /** Caller's own pre-TurnStart capture of an "auth"-classified
         *  failure row showing at send-time (null otherwise) — see
         *  sendMessage's doc comment. Always null for a held-message flush
         *  (mirrors initiatesTurn — a flush's guard question is about the
         *  ALREADY-active turn, not a fresh capture). */
        authFailureToPreserve: AgentFailure | null,
        /** True only for the auto-retry `retryLastTurn` fires from a
         *  recovery flow's own `onRecovered` callback. Bypasses the
         *  loginWaiting() check specifically: loginWaiting is a shared
         *  counter across all three recovery flows (relogin/useGlobalLogin/
         *  loginViaTerminal), so it can still read true here if a
         *  DIFFERENT, unrelated flow happens to be overlapping with the one
         *  that just confirmed success — that flow's own uncertainty has no
         *  bearing on whether THIS flow's confirmed credential is safe to
         *  retry on. Does not bypass canRetry()/authFailureToPreserve —
         *  both are already false/null by the time onRecovered fires (the
         *  triggering flow clears canRetry on its own click, and the
         *  onRecovered wiring dispatches FailureCleared before calling
         *  retryLastTurn), so there is nothing left to bypass for them.
         *  Codex P2 on PR #2338 (fifth re-review). Always false for a
         *  held-message flush (mirrors initiatesTurn/authFailureToPreserve —
         *  a flush is never the auto-retry path). */
        trustedAfterRecovery: boolean,
    ): Promise<void> => {
        // The pane is ALREADY showing the mount-time "Log in" bar
        // (opts.canRetry()) — sending anyway used to travel all the way
        // down to a real CLI spawn: a controller gets registered
        // unconditionally at agent-launch time (agent-model.ts's
        // ControllerResyncCommand), independent of this pane's own auth
        // check, and neither the identity-injection gate nor the backend
        // input handler re-verify auth before spawning. That produced a
        // doomed "Working…" round-trip that only reported "Not logged
        // in" once the subprocess itself failed its own network auth
        // handshake, seconds later. Fail exactly like a synchronous
        // AgentInputCommand rejection (same PendingMessageRejected +
        // TurnStartFailed cleanup as the catch block below) since
        // that's effectively what this is — the pane already has proof
        // this would fail. See
        // docs/retro/retro-send-while-unauthenticated-2026-07-28.md.
        //
        // Gated on initiatesTurn: a held message was already accepted into
        // an active, authenticated turn's queue before this flush runs. If
        // canRetry() only flips true mid-turn, that's unrelated to whether
        // THIS already-queued message should be attempted — dropping it
        // here would silently lose it with no retry path, contrary to the
        // held-queue's own "no expiry, wait until delivered" invariant
        // above. Let it fall through to the normal AgentInputCommand
        // attempt; a genuine failure there is already handled by the catch
        // block below without cutting the active turn short (initiatesTurn
        // is false there too). Codex P1 on PR #2338.
        //
        // Also checks loginWaiting(): canRetry() alone misses the window
        // between clicking "Log in" and that attempt actually resolving —
        // relogin() clears canRetry synchronously on click, before its own
        // OAuth poll (up to 5 minutes) confirms anything. A message sent in
        // that window is just as unconfirmed as one sent before the click.
        // Codex P1 on PR #2338 (re-review).
        //
        // Also checks the caller-captured authFailureToPreserve: neither
        // canRetry nor loginWaiting reflects a mid-turn 401/403 — that's a
        // completely separate mechanism (the failure-banner's `state.failure`
        // with `data.code === "auth"`), never touched by either signal. A
        // fresh message typed right after such a failure, before clicking
        // any recovery button, would otherwise still reach AgentInputCommand
        // on the same credential that just failed. This can't be re-derived
        // live in here — TurnStart (dispatched by the caller just before
        // this call) unconditionally clears state.failure, so the caller
        // must capture it beforehand. Codex P1 on PR #2338 (second
        // re-review).
        //
        // loginWaiting() specifically (not canRetry()/authFailureToPreserve)
        // is skipped when trustedAfterRecovery is set — see its own doc
        // comment above. Codex P2 on PR #2338 (fifth re-review).
        const loginStillWaiting = opts.loginWaiting() && !trustedAfterRecovery;
        if (initiatesTurn && (opts.canRetry() || loginStillWaiting || authFailureToPreserve)) {
            opts.log("auth", "message not sent — not logged in", "warn");
            if (!authFailureToPreserve) {
                opts.setAuthNotice(
                    loginStillWaiting
                        ? "Not logged in yet — wait for the login attempt to finish, then try again."
                        : "Not logged in — click “Log in” below to continue.",
                );
            }
            opts.model.dispatchPane({
                type: "PendingMessageRejected",
                id: messageId,
            });
            if (initiatesTurn) {
                opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
            }
            if (authFailureToPreserve) {
                // Re-dispatch the SAME failure TurnStart just cleared — the
                // failure banner (and its "Login Again"/"Use existing login"
                // actions) is this pane's actual recovery path; a generic
                // authNotice that mentions a "Log in" button not even shown
                // here (canRetry() is false in this scenario) would leave
                // the user with no working recovery affordance at all. Codex
                // P1 on PR #2338 (third re-review). Dispatched AFTER
                // TurnStartFailed so turnPhase is already Idle when this
                // reducer case runs — otherwise it reads the still-Submitting
                // phase as "a turn just ended" and hops through a transient
                // Done state before TurnStartFailed settles it back to Idle.
                opts.model.dispatchPane(
                    { type: "FailureObserved", failure: authFailureToPreserve, at: Date.now() },
                    "system",
                );
            }
            return;
        }

        // Apply runtime args (permission mode, model, effort) before this turn.
        const prov = opts.provider();
        if (prov) {
            const runtimeConfig = getRuntimeConfig(opts.block()?.meta);
            const baseArgs = prov.controllerType === "persistent" && prov.persistentLaunchArgs
                ? prov.persistentLaunchArgs
                : prov.launchArgs;
            const updatedArgs = buildRuntimeArgs(baseArgs, runtimeConfig, prov.id);
            try {
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", opts.blockId),
                    meta: { "cmd:args": updatedArgs },
                });
            } catch (err) {
                opts.log("error", `Failed to update runtime args: ${err}`, "error");
            }
        }

        // Await the send so callers can sequence multiple deliveries (the flush
        // loop relies on this to preserve submission order — ReAgent P2 on
        // PR #1484). The cmd:args round-trip above already completed, so the 30s
        // expiry below still measures backend acceptance time. Codex P2 on PR #752.
        try {
            await RpcApi.AgentInputCommand(TabRpcClient, {
                blockid: opts.blockId,
                message,
                message_id: messageId,
            });
        } catch (err: any) {
            opts.log("error", err?.message ?? String(err), "error");
            // RPC outright failed — remove the pending entry so the user
            // doesn't see a ghost row for a message the backend never received.
            opts.model.dispatchPane({
                type: "PendingMessageRejected",
                id: messageId,
            });
            // handleSendMessage (agent-view.tsx) dispatches TurnStart
            // OPTIMISTICALLY, before this RPC call even runs — a synchronous
            // AgentInputCommand failure (no controller registered for this
            // block, e.g. after a backend restart before the pane is
            // reopened; the identity spawn gate blocking on a bad
            // credential; any network-level rejection) otherwise left the
            // pane showing "Working…" forever with no path back to Idle:
            // PendingMessageRejected only ever removed the ghost pending
            // row above, never touched turnPhase. Only revert when this
            // send is what started the turn — see initiatesTurn's doc
            // comment.
            //
            // TurnStartFailed, NOT TurnReset: this used to reuse TurnReset
            // (the bang-command / handled-slash-command "no real turn
            // happened" paths above in sendMessage() still do), but
            // TurnReset is a deliberate wholesale session wipe — it also
            // clears sessionStats/sessionTotals/lastContextTokens. A
            // transient send failure on an agent with prior completed turns
            // must not wipe that accumulated history; TurnStartFailed
            // touches only turnPhase. reagent/codex P2 on PR #2318.
            if (initiatesTurn) {
                opts.model.dispatchPane({ type: "TurnStartFailed" }, "system");
            }
            return;
        }

        if (!armExpiry) return;
        // Pending acceptance timeout (issue #728 gap 2) — only for idle sends,
        // where a missing `agent-message-accepted` within PENDING_TIMEOUT_MS
        // means the delivery was lost. Held (queued-while-busy) messages get NO
        // expiry; they wait until flushed. Cleared on pane unmount.
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
    const flushHeldMessages = async (): Promise<void> => {
        // Single-flight: if a flush is already draining, let it finish — its
        // loop re-checks the queue each iteration, so any message queued while
        // it runs is picked up in order. A concurrent flush would let two
        // AgentInputCommand chains interleave and reorder sends. P2 on PR #1484.
        if (flushing) return;
        flushing = true;
        try {
            // Drain one at a time, awaiting each delivery (incl. its cmd:args
            // round-trip) so messages reach the CLI's stdin in submission order.
            // shift() (not a snapshot) so items queued mid-flush are included.
            while (heldQueue.length > 0) {
                const item = heldQueue.shift()!;
                if (item.authWasKnownBadAtQueueTime) {
                    // This item was queued while the pane already knew it was
                    // logged out (or a recovery attempt was still unconfirmed)
                    // — deliverToBackend's guard never sees it, since it's
                    // deliberately skipped for every flushed item (see the
                    // push-site comment). Reject it directly instead of
                    // trusting the same "already-active turn" reasoning that
                    // correctly applies to the OTHER items in this queue.
                    // Does not touch turnPhase (mirrors deliverToBackend's own
                    // initiatesTurn=false handling) — a real turn is still
                    // genuinely active for whatever put this pane in the
                    // "busy" branch to begin with. Codex P2 on PR #2338
                    // (sixth re-review).
                    opts.log("auth", "held message not sent — not logged in", "warn");
                    opts.model.dispatchPane({ type: "PendingMessageRejected", id: item.id });
                    continue;
                }
                await deliverToBackend(item.text, item.id, /* armExpiry */ false, /* initiatesTurn */ false, /* authFailureToPreserve */ null, /* trustedAfterRecovery */ false);
            }
        } finally {
            flushing = false;
        }
    };

    /**
     * Pop the most-recently held message off the queue, remove its pending
     * entry, and return its text so the composer can restore it (ArrowUp
     * un-queue). Null if nothing is held. The message was never sent.
     */
    const recallLatestHeld = (): { text: string } | null => {
        const item = heldQueue.pop();
        if (!item) return null;
        opts.model.dispatchPane({ type: "PendingMessageRejected", id: item.id });
        return { text: item.text };
    };

    const hasHeldMessages = (): boolean => heldQueue.length > 0;

    // Delegate to the model so the pane-frame header button and any other
    // call sites go through a single implementation.
    const back = async (): Promise<void> => {
        await opts.backToPicker();
    };

    const stopAgent = (): void => {
        // Only attempt a stop when a turn is actually in flight. Pressing Esc on
        // an empty composer when the agent is idle/finished/disconnected should
        // be a quiet no-op — not a SIGINT that lands nowhere and logs a spurious
        // "stop failed" warning. RequestStop is already a no-op off a working
        // phase; this also suppresses the unnecessary control RPC.
        const phase = paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" as const };
        if (!workingFromPhase(phase)) return;

        // Transition turnPhase → Interrupting; the status line renders
        // "Stopping…" immediately (via the turnPhase.kind ===
        // "Interrupting" binding in agent-view.tsx). `useAgentStream`
        // owns the finalization: when `session_end` arrives it
        // dispatches TurnEnd (moving phase to Done.stopped) and
        // appends the "⏹ Interrupted" row, and it also runs a fallback
        // timer that does the same cleanup if `session_end` never
        // arrives (killing a subprocess prevents the CLI from emitting
        // its own terminating result event).
        // Soft variant — cascade-during-dispatch could dispose the pane
        // before this fires; retro 2026-05-23 (agent-pane cascade →
        // replaceChild quick-win).
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
    };
}

/**
 * Run a shell command in the agent's working directory via the `shellexec`
 * RPC and surface stdout/stderr in the launch log.
 *
 * Called when the user prefixes a composer message with `!`. The blockId is
 * passed explicitly (rather than via ctx) because ctx.blockId is a string and
 * we need it for the RPC payload; ctx provides only the log sink here.
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
