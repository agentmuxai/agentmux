// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentControlBar - Collapsible runtime controls for permission mode,
 * model, and effort level. Changes take effect on the next turn.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import * as WOS from "@/app/store/wos";
import type { AgentRuntimeConfig, PermissionMode, ModelChoice, EffortLevel } from "../types";
import { DEFAULT_RUNTIME_CONFIG } from "../types";
import { getRuntimeConfig } from "../buildRuntimeArgs";

interface AgentControlBarProps {
    blockId: string;
    blockAtom: () => Block | undefined;
    providerId: string;
}

const PERMISSION_LABELS: Record<PermissionMode, string> = {
    bypass: "Bypass",
    auto: "Auto",
    acceptEdits: "Accept Edits",
    plan: "Plan",
    default: "Default",
};

const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color, #ef4444)",
    auto: "var(--accent-color, #3b82f6)",
    acceptEdits: "var(--warning-color, #eab308)",
    plan: "var(--success-color, #22c55e)",
    default: "var(--main-text-color)",
};

const MODEL_LABELS: Record<string, string> = {
    "": "Default",
    opus: "Opus",
    sonnet: "Sonnet",
    haiku: "Haiku",
};

const EFFORT_LABELS: Record<string, string> = {
    "": "Default",
    low: "Low",
    medium: "Medium",
    high: "High",
    max: "Max",
};

export const AgentControlBar = ({ blockId, blockAtom, providerId }: AgentControlBarProps): JSX.Element => {
    const [expanded, setExpanded] = createSignal(false);

    const runtime = (): AgentRuntimeConfig => getRuntimeConfig(blockAtom()?.meta);

    const isNonDefault = (): boolean => {
        const r = runtime();
        return (
            r.permissionMode !== DEFAULT_RUNTIME_CONFIG.permissionMode ||
            r.model !== DEFAULT_RUNTIME_CONFIG.model ||
            r.effort !== DEFAULT_RUNTIME_CONFIG.effort
        );
    };

    const updateRuntime = async (patch: Partial<AgentRuntimeConfig>) => {
        const current = runtime();
        const updated = { ...current, ...patch };
        const oref = WOS.makeORef("block", blockId);
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref,
            meta: { "agent:runtime": updated },
        });
    };

    const compactSummary = (): string => {
        const r = runtime();
        const parts: string[] = [PERMISSION_LABELS[r.permissionMode]];
        if (r.model) parts.push(MODEL_LABELS[r.model] || r.model);
        if (r.effort) parts.push(`Effort: ${EFFORT_LABELS[r.effort] || r.effort}`);
        return parts.join(" · ");
    };

    // Only show for Claude provider (Phase 1)
    if (providerId !== "claude") return null;

    return (
        <div class="agent-control-bar">
            <div
                class="agent-control-bar-header"
                onClick={() => setExpanded(!expanded())}
            >
                <span class="agent-control-chevron">{expanded() ? "▾" : "▸"}</span>
                <span class="agent-control-summary">
                    <Show when={!expanded()}>
                        {compactSummary()}
                        <Show when={isNonDefault()}>
                            <span class="agent-control-nondefault" title="Non-default settings active">*</span>
                        </Show>
                    </Show>
                    <Show when={expanded()}>
                        Controls
                    </Show>
                </span>
            </div>
            <Show when={expanded()}>
                <div class="agent-control-bar-body">
                    <div class="agent-control-row">
                        <label class="agent-control-label">Mode</label>
                        <select
                            class="agent-control-select"
                            value={runtime().permissionMode}
                            style={{ "border-left": `3px solid ${PERMISSION_COLORS[runtime().permissionMode]}` }}
                            onChange={(e) => updateRuntime({ permissionMode: e.target.value as PermissionMode })}
                        >
                            <option value="bypass">Bypass (no prompts)</option>
                            <option value="auto">Auto (AI classifier)</option>
                            <option value="acceptEdits">Accept Edits</option>
                            <option value="plan">Plan (read-only)</option>
                            <option value="default">Default (prompt all)</option>
                        </select>
                    </div>
                    <div class="agent-control-row">
                        <label class="agent-control-label">Model</label>
                        <select
                            class="agent-control-select"
                            value={runtime().model ?? ""}
                            onChange={(e) => updateRuntime({ model: (e.target.value || null) as ModelChoice })}
                        >
                            <option value="">Default</option>
                            <option value="opus">Opus</option>
                            <option value="sonnet">Sonnet</option>
                            <option value="haiku">Haiku</option>
                        </select>
                    </div>
                    <div class="agent-control-row">
                        <label class="agent-control-label">Effort</label>
                        <select
                            class="agent-control-select"
                            value={runtime().effort ?? ""}
                            onChange={(e) => updateRuntime({ effort: (e.target.value || null) as EffortLevel })}
                        >
                            <option value="">Default</option>
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                            <option value="max">Max (Opus only)</option>
                        </select>
                    </div>
                </div>
            </Show>
        </div>
    );
};
