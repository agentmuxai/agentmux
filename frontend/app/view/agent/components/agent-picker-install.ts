// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Which agent CLIs are installed (and how far each is from its pin), per template card: one shared check per
// provider, asked again when another picker or the Toolchain pane forgets an answer. Split out of AgentPicker.tsx
// (#13044 S6).

import type { AgentDefinition } from "@/app/store/rpc-api";
import { createEffect, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { checkProviderInstalled, invalidateInstallCheck, onInstallCheckInvalidated } from "../install-check-cache";
import { getProvider } from "../providers";
import { resolveDrift, type PinDrift } from "../providers/version-drift";

export function useInstallStates(agents: Accessor<AgentDefinition[]>) {
    // Per-agent install state, keyed by agent.id: the same answer the
    // Toolchain pane shows (one `install.check`, one backend lookup).
    //   undefined = not yet checked
    //   true      = found: AgentMux's install, or the user's own copy of a
    //               CLI AgentMux doesn't install
    //   false     = not found — card shows the bottom-right ribbon
    const [installState, setInstallState] = createSignal<Record<string, boolean | undefined>>({});
    /** Per-definition CLI drift, resolved alongside the install check (same
     *  round-trip). Absent = not checked yet, or a provider with no pin. */
    const [driftState, setDriftState] = createSignal<Record<string, PinDrift | undefined>>({});

    /** Each checked agent's resolved provider, to re-check only the cards an
     *  invalidation concerns. */
    const providerOfAgent = new Map<string, string>();
    const checkInstalled = async (agent: AgentDefinition) => {
        // Same resolver as the launch (agent-model.ts), so the CLI checked
        // here is the one that will run (#2594).
        const providerId = await resolveEffectiveLaunchProvider(agent);
        const prov = getProvider(providerId);
        if (prov) providerOfAgent.set(agent.id, prov.id);
        if (!prov) {
            setInstallState((s) => ({ ...s, [agent.id]: undefined }));
            return;
        }
        try {
            // One check per provider CLI, shared across agents, pickers and
            // the Toolchain pane. A CLI AgentMux doesn't install is checked
            // too: found on PATH or in its installer's folder, or the ribbon.
            const r = await checkProviderInstalled({
                providerId: prov.id,
                cliCommand: prov.cliCommand,
                // Lets the same round-trip answer "which version", so the card
                // can show an upgrade hint without a second probe per card.
                npmPackage: prov.npmPackage || undefined,
            });
            setInstallState((s) => ({ ...s, [agent.id]: r.installed }));
            // Same predicate the Toolchain pane renders — the card must not
            // develop its own opinion about what "behind" means.
            setDriftState((s) => ({
                ...s,
                [agent.id]: resolveDrift({
                    installed: r.version ?? undefined,
                    pinned: prov.pinnedVersion,
                    found: r.installed,
                }).drift,
            }));
        } catch {
            // Treat as needs-install — better to over-prompt than miss.
            setInstallState((s) => ({ ...s, [agent.id]: false }));
        }
    };

    /** Check `agent`'s CLI again, for every card and the Toolchain pane. */
    const recheckInstalled = async (agent: AgentDefinition) => {
        const providerId =
            providerOfAgent.get(agent.id) ?? getProvider(await resolveEffectiveLaunchProvider(agent))?.id;
        if (providerId) invalidateInstallCheck(providerId);
        await checkInstalled(agent);
    };

    // Refresh install state whenever the agent list changes.
    // Session-zone probes were removed in the Phase 1 cleanup
    // (reagent P2 on #1011) — templates carry `hasCurrentSession={false}`
    // by invariant and my-agent rows source from `MyAgentsList` which
    // doesn't need per-row session probes.
    //
    // Tracks `agents()` only. It used to track `installState()` as well,
    // and an agent's entry is only written once its check returns, so
    // every returning check re-ran this and re-issued the check for every
    // agent still pending: ~n²/2 calls, ~700 with ~37 agents
    // (docs/analysis/ANALYSIS_NEW_WINDOW_TAB_LATENCY_2026_09_30.md).
    const installCheckRequested = new Set<string>();
    createEffect(() => {
        for (const agent of agents()) {
            if (installCheckRequested.has(agent.id)) continue;
            installCheckRequested.add(agent.id);
            if (!(agent.id in untrack(installState))) void checkInstalled(agent);
        }
    });
    // Another picker or the Toolchain pane forgot an answer (an install
    // finished, a refresh found a CLI): ask again for the cards it concerns,
    // so a card never keeps a stale ribbon. One shared check per provider.
    onCleanup(
        onInstallCheckInvalidated((providerId) => {
            for (const agent of untrack(agents)) {
                const known = providerOfAgent.get(agent.id);
                if (providerId == null || known === undefined || known === providerId) void checkInstalled(agent);
            }
        })
    );

    return { installState, driftState, providerOfAgent, checkInstalled, recheckInstalled };
}

export type InstallStates = ReturnType<typeof useInstallStates>;
