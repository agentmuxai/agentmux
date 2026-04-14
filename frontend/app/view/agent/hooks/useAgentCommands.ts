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

import { type Accessor, createMemo } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import * as WOS from "@/app/store/wos";
import { buildRuntimeArgs, getRuntimeConfig } from "../buildRuntimeArgs";
import { dispatchSlashCommand } from "../commands/dispatch";
import { buildRegistry } from "../commands/registry";
import type { SlashCommandContext } from "../commands/types";
import type { ProviderDefinition } from "../providers";
import type { SignalPair } from "../state";
import type { DocumentNode } from "../types";
import type { LogFn } from "./useAgentControllerStatus";

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
}

export interface UseAgentCommands {
    /** Send a user message. Handles `/login` and `/clear` as special cases. */
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
}

// Runtime-config + auth slash commands (/model /effort /permission-mode
// /bypass /plan /runtime /login /clear) are now data-driven via
// `frontend/app/view/agent/commands/`. See
// specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md for the design.
// sendMessage below dispatches through the registry; adding a new
// command is one file create in `commands/global/` or
// `commands/providers/`, not an edit here.

export function useAgentCommands(opts: UseAgentCommandsOptions): UseAgentCommands {
    const [, setDocument] = opts.documentAtom;

    // Registry is rebuilt whenever the provider changes so
    // provider-scoped commands swap in/out. Global commands are
    // registered first and can't be shadowed (see registry.register).
    const registry = createMemo(() => buildRegistry(opts.provider()));

    const sendMessage = async (message: string): Promise<void> => {
        setDocument((prev) => [
            ...prev,
            {
                type: "user_message",
                id: `user_${Date.now()}`,
                message,
                timestamp: Date.now(),
                collapsed: false,
                summary: "",
            } as DocumentNode,
        ]);

        // Defer the scroll-to-bottom by one animation frame so the
        // user_message node has a chance to mount in the DOM before the
        // scroll math runs. Calling it synchronously lands the scroll
        // ABOVE the new message because scrollHeight doesn't include the
        // unmounted node yet. See SPEC_AGENT_PANE_FOLLOWUPS item #1.
        if (opts.onSent) {
            requestAnimationFrame(() => opts.onSent?.());
        }

        // Intercept slash commands via the registry. Claude runs in
        // stream-json mode so the CLI doesn't see these as control
        // commands — we handle them client-side. Unknown `/foo` falls
        // through to AgentInputCommand via the "passthrough" outcome.
        const trimmed = message.trim();
        if (trimmed.startsWith("/")) {
            const ctx: SlashCommandContext = {
                blockId: opts.blockId,
                provider: opts.provider,
                block: opts.block,
                documentAtom: opts.documentAtom,
                log: opts.log,
                setAuthUrl: opts.setAuthUrl,
            };
            const outcome = await dispatchSlashCommand(trimmed, registry(), ctx);
            if (outcome.kind === "handled") return;
        }

        // Apply runtime args (permission mode, model, effort) before this turn.
        const prov = opts.provider();
        if (prov) {
            const runtimeConfig = getRuntimeConfig(opts.block()?.meta);
            const baseArgs = prov.controllerType === "persistent" && prov.persistentLaunchArgs
                ? prov.persistentLaunchArgs
                : prov.launchArgs;
            const updatedArgs = buildRuntimeArgs(baseArgs, runtimeConfig);
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
        }).catch((err) => {
            opts.log("error", err?.message ?? String(err), "error");
        });
    };

    // Delegate to the model so the pane-frame header button and any other
    // call sites go through a single implementation.
    const back = async (): Promise<void> => {
        await opts.backToPicker();
    };

    const stopAgent = (): void => {
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: opts.blockId,
            signame: "SIGINT",
        }).catch((err) => {
            opts.log("warn", `stop failed: ${err?.message ?? String(err)}`, "warn");
        });
    };

    return { sendMessage, back, stopAgent };
}
