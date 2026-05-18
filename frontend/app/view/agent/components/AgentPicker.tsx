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

import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { useTabModal } from "@/app/tab/tab-modal";
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

    // Open the launch modal. Extracted so the install-modal path can
    // chain into it after a successful install.
    const openLaunchModal = (agent: ForgeAgent) => {
        tabModal.open({
            kind: "launch-agent",
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
        });
    };

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

            if (installed === false) {
                tabModal.open({
                    kind: "install-agent",
                    agent,
                    originBlockId: props.model.blockId,
                    onInstalled: () => {
                        // install.start runs at provider scope, so every
                        // ForgeAgent definition that resolves to the same
                        // canonical provider is now installed — not just
                        // the one the user clicked. Mark all of them so
                        // sibling cards drop their ribbon too.
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
                        openLaunchModal(agent);
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

    return (
        <>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="agent-view">
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
                <div class="agent-view">
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
