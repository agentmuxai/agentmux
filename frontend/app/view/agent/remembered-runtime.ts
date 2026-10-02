// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Runtime menu choices an agent remembers between launches
 * (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §3).
 *
 * Pick Opus / xhigh, close the pane, reopen the agent: it used to come back on
 * Sonnet / high, because a launch from a closed agent has no live pane to read
 * and seeded the runtime from the definition and the defaults. srv keeps a
 * `last_runtime` per agent (`agentlastruntime`); it is written only when the
 * user PICKS something, so an agent that never touched the menu remembers
 * nothing and the definition's own flags keep applying.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentRuntimeConfig, EffortLevel, PermissionMode } from "./types";

const PERMISSION_MODES: readonly PermissionMode[] = ["bypass", "auto", "acceptEdits", "plan", "default"];
const EFFORT_LEVELS: readonly EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];

/**
 * The remembered settings that are still valid for this provider: a stored
 * value that is no longer a choice (the agent was redefined onto another
 * provider, a model left the catalog) is dropped, setting by setting, so what
 * is left still applies.
 */
export function parseRememberedRuntime(
    raw: string | undefined | null,
    catalogModels: readonly string[] | undefined,
): Partial<AgentRuntimeConfig> {
    if (!raw) return {};
    let value: unknown;
    try {
        value = JSON.parse(raw);
    } catch {
        return {};
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const v = value as Record<string, unknown>;
    const out: Partial<AgentRuntimeConfig> = {};
    if (typeof v.permissionMode === "string" && (PERMISSION_MODES as readonly string[]).includes(v.permissionMode)) {
        out.permissionMode = v.permissionMode as PermissionMode;
    }
    if (typeof v.effort === "string" && (EFFORT_LEVELS as readonly string[]).includes(v.effort)) {
        out.effort = v.effort as EffortLevel;
    }
    if (typeof v.model === "string" && catalogModels?.includes(v.model)) {
        out.model = v.model;
    }
    return out;
}

/** The settings of a patch that the user picked, as the stored JSON; `null` if it picked none. */
export function pickedRuntime(patch: Partial<AgentRuntimeConfig>): Record<string, string> | null {
    const out: Record<string, string> = {};
    if (patch.permissionMode !== undefined) out.permissionMode = patch.permissionMode;
    if (patch.model !== undefined) out.model = patch.model;
    if (patch.effort !== undefined) out.effort = patch.effort;
    return Object.keys(out).length > 0 ? out : null;
}

/**
 * Remember what the user just picked for `agentId`. Only the settings picked
 * are written over what was remembered, so choosing an effort does not also
 * freeze the model the definition happened to give the pane. Never throws: a
 * failure to remember must not fail the pick that already took effect.
 */
export async function rememberRuntime(agentId: string | undefined, patch: Partial<AgentRuntimeConfig>): Promise<void> {
    const picked = pickedRuntime(patch);
    if (!agentId || !picked) return;
    try {
        const current = await RpcApi.AgentLastRuntimeCommand(TabRpcClient, { id: agentId });
        let before: Record<string, string> = {};
        try {
            const parsed = current.runtime ? JSON.parse(current.runtime) : {};
            if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) before = parsed;
        } catch {
            /* an unreadable value is replaced */
        }
        await RpcApi.AgentLastRuntimeCommand(TabRpcClient, {
            id: agentId,
            runtime: JSON.stringify({ ...before, ...picked }),
        });
    } catch (err) {
        console.warn("could not remember the runtime choice for", agentId, err);
    }
}

/** What `agentId` remembers, validated against the provider's catalog; `{}` for nothing (or on error). */
export async function recallRuntime(
    agentId: string | undefined,
    catalogModels: readonly string[] | undefined,
): Promise<Partial<AgentRuntimeConfig>> {
    if (!agentId) return {};
    try {
        const r = await RpcApi.AgentLastRuntimeCommand(TabRpcClient, { id: agentId });
        return parseRememberedRuntime(r.runtime, catalogModels);
    } catch (err) {
        console.warn("could not read the remembered runtime for", agentId, err);
        return {};
    }
}
