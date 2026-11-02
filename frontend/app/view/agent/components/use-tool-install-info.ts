// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the new-agent panel's Tools step knows about each missing system tool it can install: the resolved version,
// whether the installer can't cover it after all, and which inline installers are open. Split out of
// AgentCreatePanel.tsx.

import { createEffect, createSignal, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export interface ResolvedInstallInfo {
    commandPreview: string;
    needsElevation: boolean;
    resolvedVersion?: string | null;
}

// Tool ids the backend's system-install catalog covers
// (system_install_handlers.rs); anything else gets a link only.
// SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md.
export const SYSTEM_INSTALLABLE_IDS = new Set(["git", "node", "npm", "python"]);

export function useToolInstallInfo(missing: Accessor<readonly { tool: string }[] | null>) {
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


    return { resolvedInstalls, unavailable, expanded, toggle, markUnavailable };
}
