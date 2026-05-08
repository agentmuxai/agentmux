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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import { dispatch as dispatchPane, snapshot as panePaneSnapshot } from "@/app/store/agent-pane-state-store";
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
    block: Accessor<{ meta?: Record<string, any> } | undefined>;
    provider: Accessor<ProviderDefinition | undefined>;
    documentAtom: SignalPair<DocumentNode[]>;
    log: LogFn;
    setAuthUrl: (url: string | null) => void;
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
     * Flips true when `stopAgent` fires (user pressed Esc on an empty
     * composer) and back to false when the subsequent `session_end`
     * arrives. Drives the "Stopping…" status label and the
     * "⏹ Interrupted by user" chat row appended by `useAgentStream`.
     */
    stoppingAtom?: SignalPair<boolean>;
    /**
     * Queue of messages sent to the backend but not yet accepted.
     * `sendMessage` appends here (instead of directly to the document)
     * and `useAgentStream` removes entries on `agent-message-accepted`,
     * promoting them into the document at that moment.
     */
    pendingMessagesAtom?: SignalPair<import("../state").PendingMessage[]>;
}

export interface UseAgentCommands {
    /** Send a user message. Slash commands are intercepted via the registry. */
    sendMessage: (message: string) => Promise<void>;
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
        openPicker,
        openHelp,
    });

    const completions = (prefix: string): SlashCommand[] => {
        return registry().completions(prefix, buildCommandContext());
    };

    const availableCommands = (): SlashCommand[] => {
        return registry().list(buildCommandContext());
    };

    const sendMessage = async (message: string): Promise<void> => {
        // Intercept slash commands FIRST — some (/clear, /login) are
        // handled client-side and must not touch the backend queue at
        // all. Unknown `/foo` falls through to a real turn.
        const trimmed = message.trim();
        if (trimmed.startsWith("/")) {
            const outcome = await dispatchSlashCommand(trimmed, registry(), buildCommandContext());
            if (outcome.kind === "handled") return;
        }

        // Init guard (issue #728 gap 1, codex P2 on PR #742). The
        // reducer's TurnStart handler already suppresses turnActive
        // while initPhase === "loading", but that only stops the local
        // UI state — without this check, the message still gets queued
        // into pending AND sent over AgentInputCommand. If the backend
        // accepts before InitReady fires, the accepted-event TurnStart
        // is also suppressed, leaving the UI showing no active turn
        // while the agent IS processing. Bail early here so neither
        // happens.
        const ps = panePaneSnapshot(opts.blockId);
        if (ps?.initPhase === "loading") {
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
        dispatchPane(
            opts.blockId,
            {
                type: "PendingMessageQueued",
                id: messageId,
                text: message,
                at: Date.now(),
            },
            "user",
        );

        // Pending acceptance timeout (issue #728 gap 2). If the backend
        // never echoes `agent-message-accepted` for this id within
        // PENDING_TIMEOUT_MS, the reducer removes the ghost entry. Idempotent
        // when the message lands first — the entry is already gone and
        // PendingMessageExpired is a no-op.
        // Tracked + cleared on pane unmount to avoid dispatching against
        // an unregistered slot (which throws). PR #742 ReAgent P1.
        const expiryId = setTimeout(() => {
            pendingExpiryTimers.delete(expiryId);
            dispatchPane(opts.blockId, {
                type: "PendingMessageExpired",
                id: messageId,
            });
        }, PENDING_TIMEOUT_MS);
        pendingExpiryTimers.add(expiryId);

        // Defer the scroll-to-bottom by one animation frame so the
        // pending row has a chance to mount before the scroll math runs.
        if (opts.onSent) {
            requestAnimationFrame(() => opts.onSent?.());
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

        RpcApi.AgentInputCommand(TabRpcClient, {
            blockid: opts.blockId,
            message,
            message_id: messageId,
        }).catch((err) => {
            opts.log("error", err?.message ?? String(err), "error");
            // RPC outright failed — remove the pending entry so the user
            // doesn't see a ghost row for a message the backend never
            // received. Log already surfaces the error elsewhere.
            dispatchPane(opts.blockId, {
                type: "PendingMessageRejected",
                id: messageId,
            });
        });
    };

    // Delegate to the model so the pane-frame header button and any other
    // call sites go through a single implementation.
    const back = async (): Promise<void> => {
        await opts.backToPicker();
    };

    const stopAgent = (): void => {
        // Flip stopping → status line renders "Stopping…" immediately.
        // `useAgentStream` owns the finalization: when `session_end`
        // arrives it clears stopping + appends the "⏹ Interrupted" row,
        // and it also runs a fallback timer that does the same cleanup
        // if `session_end` never arrives (killing a subprocess prevents
        // the CLI from emitting its own terminating result event).
        dispatchPane(opts.blockId, { type: "RequestStop", at: Date.now() }, "user");
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: opts.blockId,
            signame: "SIGINT",
        }).catch((err) => {
            opts.log("warn", `stop failed: ${err?.message ?? String(err)}`, "warn");
            dispatchPane(opts.blockId, { type: "StopFailed" });
        });
    };

    return {
        sendMessage,
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
