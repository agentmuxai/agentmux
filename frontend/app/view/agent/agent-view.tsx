// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider, type ProviderDefinition } from "./providers";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { BlockService } from "@/app/store/services";
import { staticTabId } from "@/app/store/global";
import { ContextMenuModel } from "@/app/store/contextmenu";
import "./agent-view.scss";

// ── useForgeAgents hook ───────────────────────────────────────────────────────

function useForgeAgents(): () => ForgeAgent[] {
    const [agents, setAgents] = createSignal<ForgeAgent[]>([]);

    onMount(() => {
        let cancelled = false;

        async function load() {
            try {
                const result = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
                if (!cancelled) setAgents(result ?? []);
            } catch {
                // silently ignore
            }
        }

        load();

        const unsub = waveEventSubscribe({
            eventType: "forgeagents:changed",
            handler: () => load(),
        });

        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });

    return agents;
}

/**
 * Top-level wrapper — switches between agent picker and presentation view.
 */
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];

    return (
        <Show
            when={agentId()}
            fallback={<AgentPicker model={model} />}
        >
            <AgentPresentationView model={model} agentId={agentId()} />
        </Show>
    );
};

AgentViewWrapper.displayName = "AgentViewWrapper";

// ── Agent Picker ────────────────────────────────────────────────────────────────

const AgentPicker = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const agents = useForgeAgents();

    const handleSelect = async (agent: ForgeAgent) => {
        setLaunching(agent.id);
        try {
            await model.launchForgeAgent(agent);
        } catch {
            // model logs internally
        } finally {
            setLaunching(null);
        }
    };

    const busy = () => launching() !== null;

    return (
        <Show
            when={agents().length > 0}
            fallback={
                <div class="agent-view">
                    <div class="agent-picker-empty">
                        <div class="agent-picker-empty-icon">{"\u2726"}</div>
                        <div class="agent-picker-empty-title">No agents configured</div>
                        <div class="agent-picker-empty-desc">Create an agent in the Forge to get started.</div>
                        <button class="agent-picker-forge-btn" disabled>
                            + Create an agent in the Forge
                        </button>
                    </div>
                </div>
            }
        >
            <div class="agent-view">
                <div class="agent-picker">
                    <div class="agent-picker-list">
                        <For each={agents()}>
                            {(agent) => (
                                <button
                                    class={`agent-card${launching() === agent.id ? " agent-card--launching" : ""}`}
                                    onClick={() => handleSelect(agent)}
                                    disabled={busy()}
                                >
                                    <span class="agent-card-icon">{agent.icon}</span>
                                    <span class="agent-card-info">
                                        <span class="agent-card-name">{agent.name}</span>
                                        <Show when={agent.description}>
                                            <span class="agent-card-desc">{agent.description}</span>
                                        </Show>
                                    </span>
                                    <Show when={launching() === agent.id}>
                                        <span class="agent-card-spinner" />
                                    </Show>
                                </button>
                            )}
                        </For>
                    </div>
                    <div class="agent-picker-footer">
                        <button class="agent-picker-forge-btn" disabled>
                            + New agent in Forge
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

AgentPicker.displayName = "AgentPicker";

// ── Launch Flow ──────────────────────────────────────────────────────────────────

type LogFn = (tag: string, text: string, level?: "info" | "error" | "warn") => void;

/**
 * Full agent launch flow: CLI detection → auth check → controller registration.
 * Emits terminal-style log lines at each phase.
 */
async function runLaunchFlow(
    blockId: string,
    provider: ProviderDefinition | undefined,
    log: LogFn,
): Promise<void> {
    if (!provider) {
        log("error", "no provider definition — cannot resolve CLI", "error");
        return;
    }

    const oref = WOS.makeORef("block", blockId);

    // Phase 1: CLI Detection / Installation
    log("cli", `checking for ${provider.cliCommand}...`);
    let cliResult: ResolveCliResult;
    try {
        cliResult = await RpcApi.ResolveCliCommand(TabRpcClient, {
            provider_id: provider.id,
            cli_command: provider.cliCommand,
            npm_package: provider.npmPackage,
            pinned_version: provider.pinnedVersion,
            windows_install_command: provider.windowsInstallCommand,
            unix_install_command: provider.unixInstallCommand,
        }, { timeout: 120000 });
    } catch (err: any) {
        const msg = err?.message ?? String(err);
        log("cli", msg, "error");
        log("error", `${provider.cliCommand} not available — install manually or check your internet connection`, "error");
        return;
    }

    if (cliResult.source === "installed") {
        log("cli", `installed ${provider.npmPackage} (${cliResult.version})`);
    } else if (cliResult.source === "local_install") {
        log("cli", `found: ${cliResult.cli_path} (${cliResult.version}) [local install]`);
    } else {
        log("cli", `found: ${cliResult.cli_path} (${cliResult.version})`);
    }

    // Update block meta with resolved absolute path
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref,
        meta: { cmd: cliResult.cli_path },
    });

    // Phase 2: Auth Check
    log("auth", `checking ${provider.cliCommand} authentication...`);
    try {
        const authResult = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
            cli_path: cliResult.cli_path,
            auth_check_args: provider.authCheckCommand,
        }, { timeout: 30000 });
        if (authResult.authenticated) {
            const emailPart = authResult.email ? ` as ${authResult.email}` : "";
            const methodPart = authResult.auth_method ? ` (${authResult.auth_method})` : "";
            log("auth", `authenticated${emailPart}${methodPart}`);
        } else {
            log("auth", "not authenticated", "warn");
            log("auth", `run: ${provider.cliCommand} ${provider.authLoginCommand.join(" ")}`, "warn");
        }
    } catch (err: any) {
        log("auth", `check failed: ${err?.message ?? String(err)}`, "warn");
        log("auth", "authentication status unknown — will attempt anyway", "warn");
    }

    // Phase 3: Controller Registration
    log("controller", "registering subprocess controller...");
    try {
        await RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: staticTabId(),
            blockid: blockId,
            forcerestart: false,
        });
        const rts = await BlockService.GetControllerStatus(blockId);
        const status = rts?.shellprocstatus ?? "init";
        log("controller", `status: ${status}`);
        if (status === "init") {
            log("agent", "ready — type a message below to start");
        } else if (status === "done") {
            log("agent", "previous turn complete — send a message to continue");
        }
    } catch (err: any) {
        log("controller", `resync failed: ${err?.message ?? String(err)}`, "warn");
        log("agent", "ready — type a message below to start");
    }
}

// ── Presentation View ───────────────────────────────────────────────────────────

const AgentPresentationView = ({ model, agentId }: { model: AgentViewModel; agentId: string }): JSX.Element => {
    const block = model.blockAtom;
    const providerKey = (): string => block()?.meta?.["agentProvider"] ?? agentId;
    const provider = () => getProvider(providerKey());
    const outputFormat = (): string => block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json";

    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Accumulated terminal-style log lines
    type LogLine = { tag: string; text: string; level?: "info" | "error" | "warn" };
    const [logLines, setLogLines] = createSignal<LogLine[]>([]);
    const log = (tag: string, text: string, level?: "info" | "error" | "warn") => {
        setLogLines((prev) => [...prev, { tag, text, level: level ?? "info" }]);
    };

    onMount(() => {
        const name = block()?.meta?.["agentName"] ?? agentId;
        const prov = provider();
        const provName = prov?.displayName ?? providerKey();
        const cwd = block()?.meta?.["cmd:cwd"] ?? "";

        log("agent", `${name} selected (provider: ${provName})`);
        if (cwd) log("env", `working directory: ${cwd}`);

        // Full launch flow: CLI resolution → auth check → controller registration
        (async () => {
            try {
                await runLaunchFlow(model.blockId, prov, log);
            } catch (err: any) {
                log("error", err?.message ?? String(err), "error");
            }
        })();

        // Subscribe to status changes
        const unsub = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", model.blockId),
            handler: (event) => {
                const status = (event as any)?.data?.shellprocstatus;
                if (status === "running") {
                    log("subprocess", "spawned, waiting for response...");
                } else if (status === "done") {
                    const exitCode = (event as any)?.data?.shellprocexitcode;
                    if (exitCode != null && exitCode !== 0) {
                        log("subprocess", `exited with code ${exitCode}`, "error");
                    } else {
                        log("subprocess", "turn complete");
                    }
                }
            },
        });
        onCleanup(() => unsub());
    });

    // Subscribe to subprocess output and parse into DocumentNodes
    useAgentStream({
        blockId: model.blockId,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        enabled: true,
    });

    // Send user message
    const handleSendMessage = (message: string) => {
        // Add user message as a document node so it appears in the chat
        const [, setDocument] = agentAtoms().documentAtom;
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

        RpcApi.AgentInputCommand(TabRpcClient, {
            blockid: model.blockId,
            message: message,
        }).catch((err) => {
            const errMsg = err?.message ?? String(err);
            log("error", errMsg, "error");
        });
    };

    const handleBack = async () => {
        const oref = WOS.makeORef("block", model.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: null,
                    agentProvider: null,
                    agentOutputFormat: null,
                    agentName: null,
                    agentIcon: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // model logs internally
        }
    };

    // Per-pane zoom: read term:zoom from block meta (same key as terminal panes)
    const zoomFactor = createMemo(() => {
        const meta = block()?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    // Context menu for copy
    const handleContextMenu = (e: MouseEvent) => {
        const sel = window.getSelection()?.toString();
        if (!sel) return; // no selection, let default behavior
        e.preventDefault();
        ContextMenuModel.showContextMenu(
            [{ label: "Copy", click: () => navigator.clipboard.writeText(sel) }],
            e,
        );
    };

    return (
        <div
            class="agent-view agent-view--presentation"
            style={{ zoom: zoomFactor() }}
            onContextMenu={handleContextMenu}
        >
            <div class="agent-pres-header">
                <span class="agent-pres-icon">{block()?.meta?.["agentIcon"] ?? "\u26A1"}</span>
                <span class="agent-pres-name">{block()?.meta?.["agentName"] ?? provider()?.displayName ?? agentId}</span>
                <button class="agent-pres-back" onClick={handleBack} title="Back to agents">
                    {"\u2715"}
                </button>
            </div>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                logLines={logLines}
            />

            <AgentFooter agentId={agentId} onSendMessage={handleSendMessage} />
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
