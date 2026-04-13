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

import type { Accessor } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import * as WOS from "@/app/store/wos";
import { getApi } from "@/app/store/global";
import { buildRuntimeArgs, getRuntimeConfig } from "../buildRuntimeArgs";
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
}

export interface UseAgentCommands {
    /** Send a user message. Handles `/login` and `/clear` as special cases. */
    sendMessage: (message: string) => Promise<void>;
    /** Return to the agent picker by clearing the agent-identity meta keys. */
    back: () => Promise<void>;
}

export function useAgentCommands(opts: UseAgentCommandsOptions): UseAgentCommands {
    const [, setDocument] = opts.documentAtom;

    const runLoginCommand = async (): Promise<void> => {
        const prov = opts.provider();
        const cliPath = opts.block()?.meta?.["cmd"] ?? "";
        if (!prov || !cliPath) {
            opts.log("error", "/login: provider or CLI path not available", "error");
            return;
        }
        opts.log("auth", "running /login via GUI flow...");
        try {
            const authEnv: Record<string, string> = {};
            const envMeta = opts.block()?.meta?.["cmd:env"];
            if (envMeta && typeof envMeta === "object") {
                for (const [k, v] of Object.entries(envMeta)) {
                    if (typeof v === "string") authEnv[k] = v;
                }
            }
            const url = await getApi().runCliLogin(cliPath, prov.authLoginCommand, authEnv);
            if (url) {
                opts.setAuthUrl(url);
                opts.log("auth", "OAuth URL captured — browser should open automatically");
                opts.log("auth", "if it didn't, copy the URL from the box above");
            } else {
                opts.log("auth", "a browser window should have opened — complete login there");
            }
            opts.log("auth", "run /cost to verify authentication once logged in");
        } catch (err: any) {
            opts.log("error", `/login failed: ${err?.message ?? String(err)}`, "error");
        }
    };

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

        const trimmed = message.trim();
        if (trimmed === "/login") {
            await runLoginCommand();
            return;
        }
        if (trimmed === "/clear") {
            setDocument([]);
            opts.log("system", "chat cleared");
            return;
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

    return { sendMessage, back };
}
