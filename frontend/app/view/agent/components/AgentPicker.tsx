// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPicker — shown when an agent pane has no agentId in block meta.
 * Lists available Forge definitions as cards; clicking a card opens
 * the Launch modal (name + runtime), which submits back through
 * `AgentViewModel.launchForgeAgent(agent, overrides)`.
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { useTabModal, type LaunchFormStateWire } from "@/app/tab/tab-modal";
import { getPlatform } from "@/util/platformutil";
import type { AgentViewModel } from "../agent-model";
import { getProvider } from "../providers";
import { AgentCard } from "./AgentCard";
import { AgentActionBar } from "./AgentActionBar";
import type { LaunchOverrides } from "./AgentLaunchModal";

// ── useForgeAgents hook ───────────────────────────────────────────────────────

/**
 * Reactive accessor for the current Forge agent list. Subscribes to
 * `forgeagents:changed` and refetches when that event fires.
 */
export function useForgeAgents(): () => ForgeAgent[] {
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

// ── AgentPicker component ───────────────────────────────────────────────────────

interface AgentPickerProps {
    model: AgentViewModel;
}

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    const agents = useForgeAgents();
    const tabModal = useTabModal();

    // Per-agent install state, keyed by agent.id.
    //   undefined = not yet checked / non-npm provider (no install needed)
    //   true      = present in the per-version cache
    //   false     = needs install — card shows the bottom-right ribbon
    const [installState, setInstallState] = createSignal<Record<string, boolean | undefined>>({});

    const checkInstalled = async (agent: ForgeAgent) => {
        const prov = getProvider(agent.provider);
        // Non-npm providers (kimi via pip, system-PATH CLIs) don't go
        // through the install modal — never show the ribbon.
        if (!prov?.npmPackage || prov.npmPackage.length === 0) {
            setInstallState((s) => ({ ...s, [agent.id]: undefined }));
            return;
        }
        try {
            const r = await RpcApi.InstallCheckCommand(TabRpcClient, {
                providerId: prov.id,
                cliCommand: prov.cliCommand,
            });
            setInstallState((s) => ({ ...s, [agent.id]: r.installed }));
        } catch {
            // Treat as needs-install — better to over-prompt than miss.
            setInstallState((s) => ({ ...s, [agent.id]: false }));
        }
    };

    // Build the launch-agent request descriptor. Separated from the
    // open call so the install→launch chain can hand the same shape
    // to `tabModal.replace()` for a crossfade (no shell teardown).
    //
    // `initialFormState` carries the user's in-progress launch-form
    // edits through the "+ New identity/memory" round-trip — name,
    // runtime, image, identity, memory. Spec: Phase β of
    // SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md; codex P2 on
    // round 7 expanded preservation from identity+memory only to the
    // full form.
    const buildLaunchRequest = (
        agent: ForgeAgent,
        initialFormState?: Partial<LaunchFormStateWire>,
    ) => ({
        kind: "launch-agent" as const,
        agent,
        originBlockId: props.model.blockId,
        onSubmit: async (overrides: LaunchOverrides) => {
            setLaunching(agent.id);
            try {
                await props.model.launchForgeAgent(agent, overrides);
                if (props.model.nodejsError) {
                    // Surface the missing-Node banner inside the
                    // picker after the layer closes the modal —
                    // matches the pre-refactor behaviour.
                    setNodejsError(props.model.nodejsError);
                    props.model.nodejsError = null;
                }
            } finally {
                setLaunching(null);
            }
        },
        initialFormState,
        onRequestNewIdentity: (current: LaunchFormStateWire) => {
            // Thread the user's whole live form snapshot through the
            // new-identity round-trip so name/runtime/image/memory
            // survive alongside the freshly-created identity id.
            tabModal.replace({
                kind: "new-identity" as const,
                originBlockId: props.model.blockId,
                onCreated: (id: string) => {
                    tabModal.replace(
                        buildLaunchRequest(agent, {
                            ...current,
                            identityId: id,
                        }),
                    );
                },
                onCancel: () => {
                    tabModal.replace(buildLaunchRequest(agent, current));
                },
            });
        },
        onRequestNewMemory: (current: LaunchFormStateWire) => {
            // Mirror of onRequestNewIdentity above — thread the live
            // form snapshot through the new-memory round-trip so the
            // user's other edits (name, runtime, image, identity)
            // survive alongside the freshly-created memory id.
            tabModal.replace({
                kind: "new-memory" as const,
                originBlockId: props.model.blockId,
                onCreated: (id: string) => {
                    tabModal.replace(
                        buildLaunchRequest(agent, {
                            ...current,
                            memoryId: id,
                        }),
                    );
                },
                onCancel: () => {
                    tabModal.replace(buildLaunchRequest(agent, current));
                },
            });
        },
    });

    const openLaunchModal = (agent: ForgeAgent) => {
        tabModal.open(buildLaunchRequest(agent));
    };

    // Extracted from the install branch of handleSelect so the
    // prereq modal's "Launch anyway" path can route to install when
    // needed. Same shape as the existing inline install request.
    const buildInstallRequest = (agent: ForgeAgent) => ({
        kind: "install-agent" as const,
        agent,
        originBlockId: props.model.blockId,
        onInstalled: (continueToLaunch: boolean) => {
            const canonical = getProvider(agent.provider)?.id ?? agent.provider;
            setInstallState((s) => {
                const next = { ...s };
                for (const a of agents()) {
                    if ((getProvider(a.provider)?.id ?? a.provider) === canonical) {
                        next[a.id] = true;
                    }
                }
                return next;
            });
            if (continueToLaunch) {
                tabModal.replace(buildLaunchRequest(agent));
            } else {
                tabModal.close();
            }
        },
    });

    // Clicking the card opens either the install or launch modal in
    // the tab-scoped layer, depending on whether the agent's CLI is
    // already installed in the per-version cache. Phase α of
    // SPEC_AGENT_INSTALL_STAGE_2026_05_17.md.
    //
    // Re-entry guard: handleSelect awaits an IPC round-trip when
    // install state is still undefined. Without the in-flight set,
    // a double-click during the await would spawn two install
    // modals — the second mount replaces the first, and its
    // onCleanup fires `install.cancel` on the session the first one
    // had just kicked off.
    // System-tool prereq probe + modal. Phase α of
    // SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md. Caches per-tool
    // probe results for the session (PATH doesn't change mid-session
    // unless the user installs a new tool — `Refresh` clears).
    const prereqCache = new Map<string, boolean>();
    const platformKey = (): "windows" | "macos" | "linux" => {
        // Read from the CEF host's authoritative `getPlatform()` IPC
        // rather than the deprecated `window.navigator.platform` —
        // reagent P2 on PR #908.
        switch (getPlatform()) {
            case "win32": return "windows";
            case "darwin": return "macos";
            default: return "linux";
        }
    };
    const probeMissingPrereqs = async (agent: ForgeAgent) => {
        const prov = getProvider(agent.provider);
        const reqs = prov?.systemPrereqs ?? [];
        if (reqs.length === 0) return [];
        const uncached = reqs.filter((r) => !prereqCache.has(r.tool));
        if (uncached.length > 0) {
            try {
                const r = await RpcApi.ResolvePrereqsCommand(TabRpcClient, {
                    tools: uncached.map((u) => u.tool),
                });
                for (const result of r.results) {
                    prereqCache.set(result.tool, result.found);
                }
            } catch {
                // If the probe fails (e.g. backend not ready), treat
                // all as found — don't block launch on probe failure.
                for (const u of uncached) prereqCache.set(u.tool, true);
            }
        }
        const platform = platformKey();
        return reqs
            .filter((r) => prereqCache.get(r.tool) === false)
            .map((r) => ({
                tool: r.tool,
                label: r.label ?? r.tool,
                installUrl: r.installUrls[platform],
                installLinkText:
                    r.installLinkText?.[platform] ?? `Install ${r.label ?? r.tool}`,
            }));
    };

    const pendingSelect = new Set<string>();
    const handleSelect = async (agent: ForgeAgent) => {
        if (pendingSelect.has(agent.id)) return;
        pendingSelect.add(agent.id);
        try {
            setNodejsError(null);
            let installed = installState()[agent.id];

            // If the bg check hasn't populated state yet (initial render
            // race, slow IPC, freshly added definition), block on a sync
            // probe for npm-backed providers before deciding the path.
            // Without this, an unchecked npm agent would fall through to
            // openLaunchModal and the launch would write a per-version
            // `.bin` path that doesn't exist on disk.
            if (installed === undefined) {
                const prov = getProvider(agent.provider);
                const isNpmInstallable = !!prov?.npmPackage && prov.npmPackage.length > 0;
                if (isNpmInstallable) {
                    await checkInstalled(agent);
                    installed = installState()[agent.id];
                }
            }

            // System-tool prereq check (e.g. Claude Code requires git
            // — anthropics/claude-code#29898). If any are missing, open
            // the prereq modal first; user can install + refresh or
            // override via Launch anyway.
            const missing = await probeMissingPrereqs(agent);
            if (missing.length > 0) {
                const proceedWithFlow = () => {
                    if (installed === false) {
                        tabModal.replace(buildInstallRequest(agent));
                    } else {
                        tabModal.replace(buildLaunchRequest(agent));
                    }
                };
                // Recursive opener so the Refresh handler on every
                // rendered modal (including the replacement after a
                // failed re-probe) gets a working re-probe wired up.
                // Reagent + codex P1/P2 on PR #908.
                const openPrereqModal = (
                    currentMissing: typeof missing,
                    op: "open" | "replace",
                ): void => {
                    const refresh = async () => {
                        for (const m of currentMissing) prereqCache.delete(m.tool);
                        const fresh = await probeMissingPrereqs(agent);
                        if (fresh.length === 0) {
                            proceedWithFlow();
                        } else {
                            openPrereqModal(fresh, "replace");
                        }
                    };
                    const req = {
                        kind: "agent-prereqs" as const,
                        agent,
                        originBlockId: props.model.blockId,
                        missing: currentMissing,
                        onRefresh: () => void refresh(),
                        onProceed: proceedWithFlow,
                        onCancel: () => { /* layer closes */ },
                    };
                    if (op === "open") tabModal.open(req);
                    else tabModal.replace(req);
                };
                openPrereqModal(missing, "open");
                return;
            }

            if (installed === false) {
                tabModal.open({
                    kind: "install-agent",
                    agent,
                    originBlockId: props.model.blockId,
                    onInstalled: (continueToLaunch: boolean) => {
                        // install.start runs at provider scope, so every
                        // ForgeAgent definition that resolves to the same
                        // canonical provider is now installed — not just
                        // the one the user clicked. Mark all of them so
                        // sibling cards drop their ribbon too. This flip
                        // runs whether the user chose Continue or Close
                        // — codex caught the bug on PR #895 where Close
                        // left the state stale.
                        const canonical = getProvider(agent.provider)?.id ?? agent.provider;
                        setInstallState((s) => {
                            const next = { ...s };
                            for (const a of agents()) {
                                if ((getProvider(a.provider)?.id ?? a.provider) === canonical) {
                                    next[a.id] = true;
                                }
                            }
                            return next;
                        });
                        if (continueToLaunch) {
                            // Crossfade install → launch (same shell;
                            // no backdrop flicker). See
                            // SPEC_MODAL_TRANSITIONS_2026_05_18.md.
                            tabModal.replace(buildLaunchRequest(agent));
                        } else {
                            // User clicked Close (or ESC/backdrop) on
                            // the success screen — state has been
                            // flipped above; just tear down the modal.
                            tabModal.close();
                        }
                    },
                });
                return;
            }
            openLaunchModal(agent);
        } finally {
            pendingSelect.delete(agent.id);
        }
    };

    const busy = () => launching() !== null;

    // Refresh install state whenever the agent list changes.
    createEffect(() => {
        for (const agent of agents()) {
            if (!(agent.id in installState())) {
                void checkInstalled(agent);
            }
        }
    });

    // Per-pane zoom — read `term:zoom` from block meta and apply CSS
    // `zoom` on the root, matching AgentPresentationView (chat mode).
    // The universal framework (Ctrl+/-/0 in keymodel, Ctrl+Wheel in
    // app.tsx) writes `term:zoom` for any block whose `viewType ===
    // "agent"`; the picker just needs to read + apply.
    const block = props.model.blockAtom;
    const zoomFactor = createMemo(() => {
        const z = block()?.meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    return (
        <>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="agent-view" style={{ zoom: zoomFactor() }}>
                        <div class="agent-picker-empty">
                            <div class="agent-picker-empty-icon">{"\u2726"}</div>
                            <div class="agent-picker-empty-title">No definitions configured</div>
                            <div class="agent-picker-empty-desc">
                                Use the Forge pane to add your first definition.
                            </div>
                        </div>
                        <AgentActionBar />
                    </div>
                }
            >
                <div class="agent-view" style={{ zoom: zoomFactor() }}>
                    <div class="agent-picker">
                        <div class="agent-picker-list">
                            <For each={agents()}>
                                {(agent) => (
                                    <AgentCard
                                        agent={agent}
                                        launching={launching() === agent.id}
                                        disabled={busy()}
                                        installed={installState()[agent.id]}
                                        onLaunch={handleSelect}
                                    />
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
                    </div>
                    <AgentActionBar />
                </div>
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
