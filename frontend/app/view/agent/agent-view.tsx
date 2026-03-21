// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider, type ProviderDefinition } from "./providers";
import { createAgentAtoms } from "./state";
import type { DocumentNode, SubagentLinkNode } from "./types";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { BlockService } from "@/app/store/services";
import { getApi, staticTabId } from "@/app/store/global";
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
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    const agents = useForgeAgents();

    const handleSelect = async (agent: ForgeAgent) => {
        setNodejsError(null);
        setLaunching(agent.id);
        try {
            await model.launchForgeAgent(agent);
            // Check if launch was blocked by missing Node.js
            if (model.nodejsError) {
                setNodejsError(model.nodejsError);
                model.nodejsError = null;
            }
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
                    <Show when={nodejsError()}>
                        <div class="agent-nodejs-notice">
                            <div class="nodejs-notice-icon">
                                <i class="fa-solid fa-circle-exclamation" />
                            </div>
                            <div class="nodejs-notice-content">
                                <div class="nodejs-notice-title">Node.js Required</div>
                                <div class="nodejs-notice-text">{nodejsError()}</div>
                                <div class="nodejs-notice-hint">
                                    After installing, restart AgentMux and try again.
                                </div>
                            </div>
                        </div>
                    </Show>
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
 *
 * Returns:
 *  "success"     — controller registered, ready
 *  "auth_failed" — login timed out or errored (retry makes sense)
 *  "fatal"       — CLI missing, Docker missing, etc. (retry won't help)
 */
async function runLaunchFlow(
    blockId: string,
    provider: ProviderDefinition | undefined,
    log: LogFn,
    setAuthUrl: (url: string | null) => void,
    isCancelled: () => boolean,
    setLoginWaiting: (v: boolean) => void,
    authEnv?: Record<string, string>,
): Promise<"success" | "auth_failed" | "fatal"> {
    if (!provider) {
        log("error", "no provider definition — cannot resolve CLI", "error");
        return "fatal";
    }

    const oref = WOS.makeORef("block", blockId);

    // Phase 0: Container agents require Docker
    const blockData = WOS.getWaveObjectAtom<Block>(oref)();
    const agentMode = blockData?.meta?.agentMode ?? "host";
    if (agentMode === "container") {
        log("docker", "container agent — checking for Docker...");
        try {
            const dockerResult = await RpcApi.ResolveCliCommand(TabRpcClient, {
                provider_id: "docker",
                cli_command: "docker",
                npm_package: "",
                pinned_version: "",
                windows_install_command: "",
                unix_install_command: "",
            }, { timeout: 10000 });
            log("docker", `found: ${dockerResult.cli_path} (${dockerResult.version})`);
        } catch {
            log("docker", "Docker is not installed", "error");
            log("docker", "Container agents require Docker Desktop to run.", "error");
            log("docker", "Install from: https://www.docker.com/products/docker-desktop/", "error");
            return "fatal";
        }
    }

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
        return "fatal";
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

    // Phase 2: Auth Check → auto-login if not authenticated
    log("auth", `checking ${provider.cliCommand} authentication...`);
    let needsLogin = false;
    try {
        const authResult = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
            cli_path: cliResult.cli_path,
            auth_check_args: provider.authCheckCommand,
            auth_env: authEnv,
        }, { timeout: 30000 });
        if (authResult.authenticated) {
            const emailPart = authResult.email ? ` as ${authResult.email}` : "";
            const methodPart = authResult.auth_method ? ` (${authResult.auth_method})` : "";
            log("auth", `authenticated${emailPart}${methodPart}`);
        } else {
            needsLogin = true;
        }
    } catch (err: any) {
        log("auth", `check failed: ${err?.message ?? String(err)}`, "warn");
        log("auth", "authentication status unknown — will attempt anyway", "warn");
    }

    if (needsLogin) {
        log("auth", "not authenticated — starting login flow...");
        try {
            // Run from Tauri host (GUI process) so the browser opens correctly on Windows.
            // Returns immediately after spawning — browser opens, frontend polls for completion.
            await getApi().runCliLogin(
                cliResult.cli_path,
                provider.authLoginCommand,
                authEnv ?? {},
            );
            log("auth", "a browser window should have opened — complete login there");

            // Poll until authenticated, cancelled, or timed out (5 minutes)
            log("auth", "waiting for login to complete...");
            setLoginWaiting(true);
            const deadline = Date.now() + 5 * 60 * 1000;
            let authenticated = false;
            while (!isCancelled() && Date.now() < deadline) {
                await new Promise<void>((r) => setTimeout(r, 2000));
                if (isCancelled()) break;
                try {
                    const recheckResult = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                        cli_path: cliResult.cli_path,
                        auth_check_args: provider.authCheckCommand,
                        auth_env: authEnv,
                    }, { timeout: 10000 });
                    if (recheckResult.authenticated) {
                        const emailPart = recheckResult.email ? ` as ${recheckResult.email}` : "";
                        log("auth", `authenticated${emailPart}`);
                        authenticated = true;
                        break;
                    }
                } catch {
                    // keep polling on transient RPC errors
                }
            }
            setLoginWaiting(false);

            // Always clear auth URL after the login attempt
            setAuthUrl(null);

            if (isCancelled()) {
                return "auth_failed";
            }

            if (!authenticated) {
                log("auth", "login timed out after 5 minutes", "error");
                log("auth", `retry: click the button below, or run '${provider.cliCommand} ${provider.authLoginCommand.join(" ")}' manually`, "warn");
                return "auth_failed";
            }
        } catch (err: any) {
            setLoginWaiting(false);
            setAuthUrl(null);
            log("auth", `login failed: ${err?.message ?? String(err)}`, "error");
            log("auth", `run: ${provider.cliCommand} ${provider.authLoginCommand.join(" ")}`, "warn");
            return "auth_failed";
        }
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

    return "success";
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

    // OAuth URL — shown prominently with a copy button when login is needed
    const [authUrl, setAuthUrl] = createSignal<string | null>(null);
    // Whether to show the retry button (after auth_failed)
    const [canRetry, setCanRetry] = createSignal(false);
    // Whether the launch flow is currently running
    const [flowRunning, setFlowRunning] = createSignal(false);
    // Whether we're specifically in the login-polling phase
    const [loginWaiting, setLoginWaiting] = createSignal(false);
    // Mutable flag for cancelling the polling loop (set by cancel or onCleanup)
    let loginCancelled = false;

    // Build auth env for a given provider
    const buildAuthEnv = async (prov: ReturnType<typeof provider>): Promise<Record<string, string> | undefined> => {
        if (!prov?.authConfigDirEnvVar || !prov?.authDirName) return undefined;
        try {
            const authDir = await getApi().ensureAuthDir(prov.id);
            const env: Record<string, string> = { [prov.authConfigDirEnvVar]: authDir };
            if (prov.authExtraEnv) Object.assign(env, prov.authExtraEnv);
            return env;
        } catch {
            return undefined; // non-fatal — fall back to default auth dir
        }
    };

    // Runs the full launch flow; can be triggered at mount time or via retry.
    const startLaunchFlow = async () => {
        if (flowRunning()) return;
        loginCancelled = false;
        setFlowRunning(true);
        setCanRetry(false);
        const prov = provider();
        try {
            const authEnv = await buildAuthEnv(prov);
            const result = await runLaunchFlow(
                model.blockId, prov, log, setAuthUrl,
                () => loginCancelled,
                setLoginWaiting,
                authEnv,
            );
            if (result === "auth_failed" && !loginCancelled) {
                setCanRetry(true);
            }
        } catch (err: any) {
            log("error", err?.message ?? String(err), "error");
        } finally {
            setFlowRunning(false);
        }
    };

    // Cancel login: stop polling and kill the background CLI process.
    const cancelLogin = () => {
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        log("auth", "login cancelled", "warn");
    };

    // If the pane is closed while login is in progress, cancel and kill the CLI process.
    onCleanup(() => {
        if (loginWaiting()) {
            loginCancelled = true;
            getApi().cancelCliLogin().catch(() => {});
        }
    });

    onMount(() => {
        const name = block()?.meta?.["agentName"] ?? agentId;
        const prov = provider();
        const provName = prov?.displayName ?? providerKey();
        const cwd = block()?.meta?.["cmd:cwd"] ?? "";

        log("agent", `${name} selected (provider: ${provName})`);
        if (cwd) log("env", `working directory: ${cwd}`);

        // Full launch flow: CLI resolution → auth check → controller registration
        startLaunchFlow();

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

        // Subscribe to subagent:spawned events — render clickable links
        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;

                const linkNode: SubagentLinkNode = {
                    type: "subagent_link",
                    id: `subagent_${data.agentId}`,
                    subagentId: data.agentId,
                    slug: data.slug ?? "",
                    parentAgent: data.parentAgent ?? "",
                    sessionId: data.sessionId ?? "",
                    status: "active",
                    model: data.model ?? null,
                };

                const [, setDoc] = agentAtoms().documentAtom;
                setDoc((prev) => [...prev, linkNode]);
                log("subagent", `spawned: ${data.slug || data.agentId}`);
            },
        });
        onCleanup(() => unsubSpawned());

        // Subscribe to subagent:completed — update link status
        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;

                const nodeId = `subagent_${data.agentId}`;
                const [, setDoc] = agentAtoms().documentAtom;
                setDoc((prev) =>
                    prev.map((n) =>
                        n.id === nodeId && n.type === "subagent_link"
                            ? { ...n, status: "completed" as const }
                            : n
                    )
                );
            },
        });
        onCleanup(() => unsubCompleted());
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

    // Handle subagent link click — open a subagent pane
    const handleSubagentClick = (node: SubagentLinkNode) => {
        if (isSubagentPaneOpen(node.subagentId)) {
            log("subagent", `pane already open for ${node.slug || node.subagentId}`);
            return;
        }
        openSubagentPane({
            subagentId: node.subagentId,
            slug: node.slug,
            parentAgent: node.parentAgent,
            parentBlockId: model.blockId,
            sessionId: node.sessionId,
        }).then((blockId) => {
            if (blockId) {
                log("subagent", `opened pane for ${node.slug || node.subagentId}`);
            }
        });
    };

    // Context menu for copy
    const handleContextMenu = (e: MouseEvent) => {
        const sel = window.getSelection()?.toString();
        if (!sel) return; // no selection, let default behavior
        e.preventDefault();
        ContextMenuModel.showContextMenu(
            [{ label: "Copy", click: () => clipboardWriteText(sel) }],
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
                authUrl={authUrl}
                onSubagentClick={handleSubagentClick}
            />

            <Show when={loginWaiting()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn agent-retry-btn--cancel" onClick={cancelLogin}>
                        Cancel Login
                    </button>
                </div>
            </Show>
            <Show when={canRetry()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn" onClick={startLaunchFlow}>
                        Retry Login
                    </button>
                </div>
            </Show>

            <AgentFooter agentId={agentId} onSendMessage={handleSendMessage} />
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
