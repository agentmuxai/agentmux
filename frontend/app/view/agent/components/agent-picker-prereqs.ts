// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The system tools a harness needs (git, node, ...) and which of them are missing, probed once per session.
// Split out of AgentPicker.tsx (#13044 S6).

import type { AgentDefinition } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getPlatform } from "@/util/platformutil";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { getProvider } from "../providers";
import { needsProbe, prereqShortfall, shortfallLabel, type PrereqProbe } from "../providers/prereq-check";

export function createPrereqs() {
    // System-tool prereq probe. Phase α of
    // SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md. Caches per-tool
    // probe results for the session (PATH doesn't change mid-session
    // unless the user installs a new tool; the panel's Check again clears).
    const prereqCache = new Map<string, PrereqProbe>();
    const platformKey = (): "windows" | "macos" | "linux" => {
        // Read from the CEF host's authoritative `getPlatform()` IPC
        // rather than the deprecated `window.navigator.platform` —
        // reagent P2 on PR #908.
        switch (getPlatform()) {
            case "win32":
                return "windows";
            case "darwin":
                return "macos";
            default:
                return "linux";
        }
    };
    const probeMissingPrereqs = async (agent: AgentDefinition) => {
        // Same resolver as the launch (#2594), so the prereqs probed are
        // those of the provider that will run.
        const providerId = await resolveEffectiveLaunchProvider(agent);
        const prov = getProvider(providerId);
        const reqs = prov?.systemPrereqs ?? [];
        if (reqs.length === 0) return [];
        const uncached = reqs.filter((r) => needsProbe(r, prereqCache.get(r.tool)));
        if (uncached.length > 0) {
            try {
                const versions = uncached.filter((u) => u.minVersion).map((u) => u.tool);
                const r = await RpcApi.ResolvePrereqsCommand(TabRpcClient, {
                    tools: uncached.map((u) => u.tool),
                    ...(versions.length > 0 ? { versions } : {}),
                });
                for (const result of r.results) {
                    const wanted = uncached.some((u) => u.tool === result.tool && u.minVersion);
                    prereqCache.set(result.tool, {
                        found: result.found,
                        version: wanted ? (result.version ?? null) : undefined,
                    });
                }
            } catch {
                // If the probe fails (e.g. backend not ready), treat
                // all as found — don't block launch on probe failure.
                for (const u of uncached) prereqCache.set(u.tool, { found: true, version: null });
            }
        }
        const platform = platformKey();
        return reqs.flatMap((r) => {
            const shortfall = prereqShortfall(r, prereqCache.get(r.tool));
            if (!shortfall) return [];
            const label = r.label ?? r.tool;
            return [
                {
                    tool: r.tool,
                    label: shortfallLabel(r, shortfall),
                    installUrl: r.installUrls[platform],
                    installLinkText:
                        shortfall.kind === "too-old"
                            ? `Update ${label}`
                            : (r.installLinkText?.[platform] ?? `Install ${label}`),
                },
            ];
        });
    };

    /** The system tools `agent`'s harness needs that aren't on PATH; `fresh` asks again. */
    const prereqsFor = async (agent: AgentDefinition, fresh: boolean) => {
        if (fresh) prereqCache.clear();
        return probeMissingPrereqs(agent);
    };

    return { prereqsFor };
}
