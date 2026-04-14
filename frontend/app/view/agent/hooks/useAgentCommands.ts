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
import type { AgentRuntimeConfig, DocumentNode, EffortLevel, ModelChoice, PermissionMode } from "../types";
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

// ── Runtime-config slash-command helpers ───────────────────────────────────
//
// The Claude Code CLI runs in non-interactive stream-json mode, so the user's
// slash commands (e.g. `/model sonnet`) reach the model as raw text instead
// of being handled by the CLI's own dispatcher. We intercept the well-known
// runtime-config commands here and map them to `block.meta["agent:runtime"]`
// mutations — the exact same path the AgentControlBar dropdowns use. Takes
// effect on the NEXT turn (runtime args are rebuilt in sendMessage below
// before each RPC invocation).
//
// Non-runtime commands are listed below as "recognized but not supported in
// stream-json mode" so the user gets a helpful error instead of silently
// sending `/memory` as a user message to the model.

const MODEL_ALIASES: Record<string, ModelChoice> = {
    "opus": "opus",
    "sonnet": "sonnet",
    "haiku": "haiku",
    "claude-opus": "opus",
    "claude-sonnet": "sonnet",
    "claude-haiku": "haiku",
    "default": null,
    "": null,
};

const EFFORT_ALIASES: Record<string, EffortLevel> = {
    "low": "low",
    "medium": "medium",
    "med": "medium",
    "high": "high",
    "max": "max",
    "default": null,
    "": null,
};

const PERMISSION_ALIASES: Record<string, PermissionMode> = {
    "bypass": "bypass",
    "dangerous": "bypass",
    "skip": "bypass",
    "dangerously-skip-permissions": "bypass",
    "auto": "auto",
    "accept": "acceptEdits",
    "acceptedits": "acceptEdits",
    "accept-edits": "acceptEdits",
    "plan": "plan",
    "default": "default",
    "": "default",
};

/** Parse `/command arg` → `["command", "arg"]`. Leading slash stripped. */
function parseSlashCommand(input: string): [string, string] {
    const trimmed = input.trim();
    if (!trimmed.startsWith("/")) return ["", ""];
    const rest = trimmed.slice(1);
    const spaceIdx = rest.indexOf(" ");
    if (spaceIdx < 0) return [rest.toLowerCase(), ""];
    return [rest.slice(0, spaceIdx).toLowerCase(), rest.slice(spaceIdx + 1).trim()];
}

/** True if `input` is a known slash command we handle client-side. */
function isRuntimeSlashCommand(name: string): boolean {
    return (
        name === "model" ||
        name === "effort" ||
        name === "permission-mode" ||
        name === "permission" ||
        name === "perm" ||
        name === "bypass" ||
        name === "plan" ||
        name === "runtime"
    );
}

export function useAgentCommands(opts: UseAgentCommandsOptions): UseAgentCommands {
    const [, setDocument] = opts.documentAtom;

    // Mutate block.meta["agent:runtime"] with a partial runtime config, the
    // same way AgentControlBar.updateRuntime does. Returns the merged config
    // on success, null on RPC failure — callers check the return value and
    // skip the user-visible confirmation if the mutation didn't actually
    // land (otherwise we log "model set to X" right alongside the error
    // from SetMetaCommand, which reagent flagged as a false-success path).
    const updateRuntime = async (patch: Partial<AgentRuntimeConfig>): Promise<AgentRuntimeConfig | null> => {
        const current = getRuntimeConfig(opts.block()?.meta);
        const updated: AgentRuntimeConfig = { ...current, ...patch };
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", opts.blockId),
                meta: { "agent:runtime": updated },
            });
            return updated;
        } catch (err: any) {
            opts.log("error", `failed to update runtime config: ${err?.message ?? String(err)}`, "error");
            return null;
        }
    };

    const handleModelCommand = async (arg: string): Promise<void> => {
        const key = arg.toLowerCase();
        if (!(key in MODEL_ALIASES)) {
            opts.log(
                "system",
                `/model: unknown model '${arg}'. Try: opus | sonnet | haiku | default`,
                "warn",
            );
            return;
        }
        const model = MODEL_ALIASES[key];
        const updated = await updateRuntime({ model });
        if (!updated) return;
        const label = updated.model ?? "default";
        opts.log("system", `model set to ${label} (applies to next turn)`);
    };

    const handleEffortCommand = async (arg: string): Promise<void> => {
        const key = arg.toLowerCase();
        if (!(key in EFFORT_ALIASES)) {
            opts.log(
                "system",
                `/effort: unknown level '${arg}'. Try: low | medium | high | max | default`,
                "warn",
            );
            return;
        }
        const effort = EFFORT_ALIASES[key];
        const updated = await updateRuntime({ effort });
        if (!updated) return;
        const label = updated.effort ?? "default";
        opts.log("system", `effort set to ${label} (applies to next turn)`);
    };

    const handlePermissionModeCommand = async (arg: string): Promise<void> => {
        const key = arg.toLowerCase();
        if (!(key in PERMISSION_ALIASES)) {
            opts.log(
                "system",
                `/permission-mode: unknown mode '${arg}'. Try: bypass | auto | accept | plan | default`,
                "warn",
            );
            return;
        }
        const mode = PERMISSION_ALIASES[key];
        const updated = await updateRuntime({ permissionMode: mode });
        if (!updated) return;
        opts.log("system", `permission mode set to ${updated.permissionMode} (applies to next turn)`);
    };

    const handleRuntimeCommand = async (): Promise<void> => {
        const r = getRuntimeConfig(opts.block()?.meta);
        const parts = [
            `permission: ${r.permissionMode}`,
            `model: ${r.model ?? "default"}`,
            `effort: ${r.effort ?? "default"}`,
        ];
        opts.log("system", `runtime config — ${parts.join(" · ")}`);
    };

    const dispatchRuntimeSlashCommand = async (name: string, arg: string): Promise<void> => {
        switch (name) {
            case "model":
                await handleModelCommand(arg);
                return;
            case "effort":
                await handleEffortCommand(arg);
                return;
            case "permission-mode":
            case "permission":
            case "perm":
                await handlePermissionModeCommand(arg);
                return;
            case "bypass": {
                // Shortcut: `/bypass` with no arg enables permission bypass;
                // `/bypass off` or `/bypass default` reverts to default. Any
                // other arg is a typo — warn instead of silently enabling
                // dangerous mode, since bypass disables permission prompts
                // and a typo like `/bypass of` shouldn't be load-bearing.
                const bypassArg = arg.toLowerCase();
                if (bypassArg === "") {
                    await handlePermissionModeCommand("bypass");
                } else if (bypassArg === "off" || bypassArg === "default") {
                    await handlePermissionModeCommand("default");
                } else {
                    opts.log(
                        "system",
                        `/bypass: unknown arg '${arg}'. Use '/bypass' to enable or '/bypass off' to disable.`,
                        "warn",
                    );
                }
                return;
            }
            case "plan":
                // Shortcut: `/plan` = permission-mode plan
                await handlePermissionModeCommand("plan");
                return;
            case "runtime":
                await handleRuntimeCommand();
                return;
        }
    };

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

        // Defer the scroll-to-bottom by one animation frame so the
        // user_message node has a chance to mount in the DOM before the
        // scroll math runs. Calling it synchronously lands the scroll
        // ABOVE the new message because scrollHeight doesn't include the
        // unmounted node yet. See SPEC_AGENT_PANE_FOLLOWUPS item #1.
        if (opts.onSent) {
            requestAnimationFrame(() => opts.onSent?.());
        }

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

        // Intercept runtime-config slash commands (/model, /effort,
        // /permission-mode, /bypass, /plan, /runtime). AgentMux drives
        // Claude in stream-json mode, so the CLI never sees these as
        // control commands — it treats them as user text and Claude
        // replies "not a command". We handle them client-side and mutate
        // block.meta["agent:runtime"] so they take effect on the NEXT
        // turn via the runtime-args rebuild below.
        if (trimmed.startsWith("/")) {
            const [name, arg] = parseSlashCommand(trimmed);
            if (isRuntimeSlashCommand(name)) {
                await dispatchRuntimeSlashCommand(name, arg);
                return;
            }
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
