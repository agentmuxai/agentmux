// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreatePanel — the panel that opens below a harness card in the agent
 * picker's New Agent grid, pushing the cards below it down
 * (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3; the pattern is P1 in
 * SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.7). It replaces the
 * three modals a new agent used to take, and shows only the steps that apply:
 *
 *  1. Tools: system tools the harness needs that aren't on PATH.
 *  2. Install: the harness's CLI, when it isn't found. AgentMux installs an
 *     npm-managed CLI here (the run outlives the panel, `install-sessions.ts`);
 *     for a CLI AgentMux doesn't install, it shows the vendor's installer.
 *  3. Create: the agent's name and settings (`AgentCreateForm`).
 */

import { createEffect, createMemo, createResource, createSignal, For, Match, onCleanup, Show, Switch, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { InstallProgress } from "@/element/install/InstallProgress";
import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { currentPlatform } from "@/app/store/toolchain-catalog";
import { SystemToolInstallInline } from "@/app/view/toolchain/SystemToolInstallInline";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { agentInstallSession, onAgentInstallDone, watchAgentInstall } from "../install-sessions";
import { getProvider } from "../providers";
import { AgentCreateForm, type CreateDraft, type CreatedAgent } from "./AgentCreateForm";
import type { AgentDefinition } from "@/app/store/rpc-api";
import "./AgentCreatePanel.scss";

/** A system tool the harness needs that wasn't found, with where to get it. */
export interface MissingPrereq {
    tool: string;
    label: string;
    installUrl: string;
    installLinkText: string;
}

interface ResolvedInstallInfo {
    commandPreview: string;
    needsElevation: boolean;
    resolvedVersion?: string | null;
}

// Tool ids the backend's system-install catalog covers
// (system_install_handlers.rs); anything else gets a link only.
// SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md.
const SYSTEM_INSTALLABLE_IDS = new Set(["git", "node", "npm", "python"]);

type Step = "tools" | "install" | "create";

interface AgentCreatePanelProps {
    template: AgentDefinition;
    draft: CreateDraft;
    /** The picker's answer for this card's CLI: false = not found. */
    installed: boolean | undefined;
    /** System tools the harness needs that aren't on PATH; `fresh` re-asks. */
    probePrereqs: (fresh: boolean) => Promise<MissingPrereq[]>;
    /** Asks the picker to check the CLI again (after an install). */
    recheckInstalled: () => Promise<void>;
    /** Launches the agent the form created. */
    onCreated: (agent: CreatedAgent) => Promise<void>;
    onClose: () => void;
    /** Creating: the panel refuses to close meanwhile. */
    onBusyChange: (busy: boolean) => void;
}

function open(url: string): void {
    try {
        getApi().openExternal(url);
    } catch {
        /* the link stays visible as text */
    }
}

export const AgentCreatePanel = (props: AgentCreatePanelProps): JSX.Element => {
    // The provider that will run, through the launch's own resolver (#2594).
    const [providerId] = createResource(() => props.template, resolveEffectiveLaunchProvider);
    const provider = () => getProvider(providerId() ?? props.template.provider);
    const name = () => getCliCatalogEntry(providerId() ?? props.template.provider)?.displayName ?? props.template.name;
    /** AgentMux doesn't install this CLI: the user installs it with its own installer. */
    const userInstalled = () => !!provider() && !provider()!.npmPackage;

    // ── Tools ────────────────────────────────────────────────────────
    const [missing, setMissing] = createSignal<MissingPrereq[] | null>(null);
    const [skipTools, setSkipTools] = createSignal(false);
    // Installed by the inline installer in this panel; srv's PATH is stale
    // until restart, so a re-check may still report them missing.
    const [installedPendingRestart, setInstalledPendingRestart] = createSignal<ReadonlySet<string>>(new Set());
    const checkTools = async (fresh: boolean) => setMissing(await props.probePrereqs(fresh));
    void checkTools(false);
    const stillMissing = createMemo(() => (missing() ?? []).filter((m) => !installedPendingRestart().has(m.tool)));

    // ── Install ──────────────────────────────────────────────────────
    const session = createMemo(() => {
        const id = providerId();
        return id && !userInstalled() ? agentInstallSession(id) : undefined;
    });
    // This panel shows the install, so its end needs no toast; an install
    // that ends here re-checks the CLI so the Create step follows.
    createEffect(() => {
        const id = providerId();
        if (!id || userInstalled()) return;
        const release = watchAgentInstall(id);
        const unsubscribe = onAgentInstallDone(id, (ok) => {
            if (ok) void props.recheckInstalled();
        });
        onCleanup(() => {
            release();
            unsubscribe();
        });
    });
    const [checkingAgain, setCheckingAgain] = createSignal(false);
    // The install finished here and the user chose Continue: go on to Create
    // even if the re-check hasn't caught up yet (the launch looks again).
    const [installDone, setInstallDone] = createSignal(false);
    const checkAgain = async () => {
        setCheckingAgain(true);
        try {
            await props.recheckInstalled();
        } finally {
            setCheckingAgain(false);
        }
    };
    const vendorCommand = () => {
        const p = provider();
        if (!p) return "";
        return currentPlatform() === "windows" ? p.windowsInstallCommand : p.unixInstallCommand;
    };

    // ── Which step ───────────────────────────────────────────────────
    const steps = createMemo<Step[]>(() => {
        const out: Step[] = [];
        if (stillMissing().length > 0 || installedPendingRestart().size > 0) out.push("tools");
        if (props.installed === false || session()?.state() === "running" || session()?.state() === "done") out.push("install");
        out.push("create");
        return out;
    });
    const step = createMemo<Step>(() => {
        if (missing() === null) return "tools";
        if (stillMissing().length > 0 && !skipTools()) return "tools";
        if ((props.installed === false && !installDone()) || session()?.state() === "running") return "install";
        return "create";
    });
    const STEP_LABEL: Record<Step, string> = { tools: "Tools", install: "Install", create: "Create" };

    // Resolved once per installable missing tool, so its button can name the
    // version before the user opens the installer.
    const [resolvedInstalls, setResolvedInstalls] = createSignal<ReadonlyMap<string, ResolvedInstallInfo>>(new Map());
    const [unavailable, setUnavailable] = createSignal<ReadonlySet<string>>(new Set());
    const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set());
    const toggle = (tool: string) =>
        setExpanded((prev) => {
            const next = new Set(prev);
            if (next.has(tool)) next.delete(tool);
            else next.add(tool);
            return next;
        });
    const markUnavailable = (tool: string) => {
        setExpanded((prev) => {
            const next = new Set(prev);
            next.delete(tool);
            return next;
        });
        setUnavailable((prev) => new Set(prev).add(tool));
    };
    const resolving = new Set<string>();
    createEffect(() => {
        for (const req of missing() ?? []) {
            if (!SYSTEM_INSTALLABLE_IDS.has(req.tool) || resolving.has(req.tool)) continue;
            resolving.add(req.tool);
            void RpcApi.ToolchainResolveInstallCommandCommand(TabRpcClient, { toolId: req.tool })
                .then((r) => {
                    if (!r.available) return markUnavailable(req.tool);
                    setResolvedInstalls((prev) =>
                        new Map(prev).set(req.tool, {
                            commandPreview: r.commandPreview,
                            needsElevation: r.needsElevation,
                            resolvedVersion: r.resolvedVersion ?? null,
                        }),
                    );
                })
                .catch(() => markUnavailable(req.tool));
        }
    });

    return (
        <section class="agent-create-panel" aria-label={`New ${props.template.name} agent`} data-testid="agent-create-panel">
            <header class="agent-create-panel-header">
                <h3 class="agent-create-panel-title">New {props.template.name} agent</h3>
                <Show when={steps().length > 1}>
                    <ol class="agent-create-steps" data-testid="agent-create-steps">
                        <For each={steps()}>
                            {(s) => (
                                <li class="agent-create-step" classList={{ "agent-create-step--current": s === step() }} aria-current={s === step() ? "step" : undefined}>
                                    {STEP_LABEL[s]}
                                </li>
                            )}
                        </For>
                    </ol>
                </Show>
            </header>

            <Switch>
                <Match when={step() === "tools"}>
                    <Show when={missing() !== null} fallback={<p class="agent-create-panel-text">Checking what {props.template.name} needs…</p>}>
                        <div class="agent-create-tools" data-testid="agent-create-tools">
                            <p class="agent-create-panel-text">
                                {props.template.name} needs tools that aren't on your PATH. Install them, then check
                                again.
                            </p>
                            <ul class="agent-create-tool-list">
                                <For each={missing() ?? []}>
                                    {(req) => (
                                        <li class="agent-create-tool-row" classList={{ "is-ok": installedPendingRestart().has(req.tool) }}>
                                            <Show
                                                when={!installedPendingRestart().has(req.tool)}
                                                fallback={
                                                    <div class="agent-create-tool-info">
                                                        <div class="agent-create-tool-tool">
                                                            {req.label}
                                                            <span class="agent-create-tool-tool-status agent-create-tool-tool-status-ok">
                                                                {" "}— installed, restart AgentMux to use it
                                                            </span>
                                                        </div>
                                                    </div>
                                                }
                                            >
                                                <div class="agent-create-tool-info">
                                                    <div class="agent-create-tool-tool">
                                                        {req.label}
                                                        <span class="agent-create-tool-tool-status"> — not found</span>
                                                    </div>
                                                    <a
                                                        class="agent-create-tool-link"
                                                        href={req.installUrl}
                                                        onClick={(e) => {
                                                            e.preventDefault();
                                                            open(req.installUrl);
                                                        }}
                                                    >
                                                        {req.installLinkText}
                                                        <span aria-hidden="true"> ↗</span>
                                                    </a>
                                                    <Show when={SYSTEM_INSTALLABLE_IDS.has(req.tool) && !unavailable().has(req.tool)}>
                                                        <Button tone="quiet" onClick={() => toggle(req.tool)}>
                                                            {(() => {
                                                                const v = resolvedInstalls().get(req.tool)?.resolvedVersion;
                                                                return v ? `Install v${v} now ↓` : "or install it now ↓";
                                                            })()}
                                                        </Button>
                                                        <Show when={expanded().has(req.tool)}>
                                                            <SystemToolInstallInline
                                                                toolId={req.tool}
                                                                resolvedInfo={resolvedInstalls().get(req.tool)}
                                                                onInstalled={() => {
                                                                    setInstalledPendingRestart((prev) => new Set(prev).add(req.tool));
                                                                    void checkTools(true);
                                                                }}
                                                                onUnavailable={() => markUnavailable(req.tool)}
                                                            />
                                                        </Show>
                                                    </Show>
                                                </div>
                                            </Show>
                                        </li>
                                    )}
                                </For>
                            </ul>
                            <p class="agent-create-tool-hint">
                                Installed somewhere AgentMux can't see? Continue anyway: your CLI might still find it.
                            </p>
                            <div class="agent-create-actions">
                                <Button tone="neutral" onClick={() => props.onClose()}>
                                    Cancel
                                </Button>
                                <Button tone="neutral" onClick={() => void checkTools(true)}>
                                    Check again
                                </Button>
                                <Button tone="accent" onClick={() => setSkipTools(true)} data-testid="agent-create-tools-continue">
                                    Continue anyway
                                </Button>
                            </div>
                        </div>
                    </Show>
                </Match>

                <Match when={step() === "install" && userInstalled()}>
                    <div class="agent-create-install" data-testid="agent-create-vendor-install">
                        <p class="agent-create-panel-text">
                            AgentMux doesn't install {name()}: install it with its own installer, which puts it on your
                            PATH, outside AgentMux's isolation. Then check again.
                        </p>
                        <Show when={vendorCommand()}>
                            <div class="agent-create-command">
                                <code data-testid="agent-create-vendor-command">{vendorCommand()}</code>
                                <Button tone="quiet" onClick={() => void clipboardWriteText(vendorCommand())} title="Copy">
                                    <i class="fa-solid fa-copy" aria-hidden="true" /> Copy
                                </Button>
                            </div>
                        </Show>
                        <Show when={provider()?.docsUrl}>
                            {(url) => (
                                <a
                                    class="agent-create-tool-link"
                                    href={url()}
                                    onClick={(e) => {
                                        e.preventDefault();
                                        open(url());
                                    }}
                                >
                                    {name()}'s install instructions<span aria-hidden="true"> ↗</span>
                                </a>
                            )}
                        </Show>
                        <div class="agent-create-actions">
                            <Button tone="neutral" onClick={() => props.onClose()}>
                                Cancel
                            </Button>
                            <Button tone="accent" busy={checkingAgain()} onClick={() => void checkAgain()} data-testid="agent-create-check-again">
                                Check again
                            </Button>
                        </div>
                    </div>
                </Match>

                <Match when={step() === "install" && session()}>
                    {(s) => (
                        <div class="agent-create-install" data-testid="agent-create-npm-install">
                            <p class="agent-create-panel-text">
                                {name()} isn't installed yet. AgentMux installs its own copy, kept apart from any you
                                installed yourself. It needs an internet connection, and keeps going if you close this.
                            </p>
                            <InstallProgress session={s()} kind="provider-cli" />
                            <div class="agent-create-actions">
                                <Switch>
                                    <Match when={s().state() === "idle"}>
                                        <Button tone="neutral" onClick={() => props.onClose()}>
                                            Cancel
                                        </Button>
                                        <Button tone="accent" onClick={() => void s().start()} data-testid="agent-create-install-start">
                                            Install now
                                        </Button>
                                    </Match>
                                    <Match when={s().state() === "running"}>
                                        <Button tone="neutral" onClick={() => void s().cancel()} data-testid="agent-create-install-cancel">
                                            Cancel install
                                        </Button>
                                    </Match>
                                    <Match when={s().state() === "failed"}>
                                        <Button tone="neutral" onClick={() => props.onClose()}>
                                            Close
                                        </Button>
                                        <Button tone="accent" onClick={() => void s().start()}>
                                            Retry
                                        </Button>
                                    </Match>
                                    <Match when={s().state() === "done"}>
                                        <Button tone="neutral" onClick={() => props.onClose()}>
                                            Close
                                        </Button>
                                        <Button
                                            tone="accent"
                                            onClick={() => {
                                                setInstallDone(true);
                                                void props.recheckInstalled();
                                            }}
                                            data-testid="agent-create-install-continue"
                                        >
                                            Continue
                                        </Button>
                                    </Match>
                                </Switch>
                            </div>
                        </div>
                    )}
                </Match>

                <Match when={step() === "create"}>
                    <AgentCreateForm
                        template={props.template}
                        draft={props.draft}
                        onCreated={props.onCreated}
                        onCancel={() => props.onClose()}
                        onBusyChange={props.onBusyChange}
                    />
                </Match>
            </Switch>
        </section>
    );
};

AgentCreatePanel.displayName = "AgentCreatePanel";
