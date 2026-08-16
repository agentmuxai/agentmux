// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Launch-environment helpers extracted from agent-model.ts (see
 * docs/specs — modularization pass, 2026-07-23). These resolve
 * host-level paths/availability needed before spawning an agent CLI.
 * No `this`/class coupling — standalone functions the model calls into.
 */

import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Logger } from "@/util/logger";

/**
 * Check if Node.js is available. Required for npm-based providers (Codex, Gemini).
 * Claude has its own standalone installer and doesn't need Node.js.
 * Returns null if Node.js is available or not needed, or an error message string.
 */
export async function checkNodejsForProvider(providerId: string): Promise<string | null> {
    if (providerId === "claude") return null; // Claude has standalone installer
    try {
        const status = await getApi().checkNodejsAvailable();
        if (!status.available || !status.npm_available) {
            const missing = !status.available ? "Node.js" : "npm";
            return `${missing} is not installed. Install Node.js from https://nodejs.org/ (LTS recommended).`;
        }
        return null;
    } catch (e) {
        Logger.warn("agent", "Failed to check Node.js availability", { error: String(e) });
        return null; // Don't block launch on check failure — let npm install fail with its own error
    }
}

/**
 * Return the AgentMux user-home base directory as an absolute path.
 *
 * Routed by the CEF host so per-agent paths (working dir, `GH_CONFIG_DIR`, …)
 * land in the right place for the instance type:
 *   - Portable: `<portable>/data`
 *   - Installed: `~/.agentmux`
 *   - `AGENTMUX_DATA_HOME` env override: wins over both.
 *
 * Falls back to `$HOME/.agentmux` only if the host IPC hasn't populated the
 * cached value yet (shouldn't happen in practice — `initCefApi` fetches it
 * before any agent launch).
 *
 * See `docs/specs/portable-agent-working-dirs.md`.
 */
export function agentmuxHome(): string {
    const fromHost = getApi().getUserHomeDir();
    if (fromHost) return fromHost;
    const home = getApi().getEnv("HOME") || getApi().getEnv("USERPROFILE") || "~";
    return `${home}/.agentmux`;
}

/**
 * Resolve the version-isolated CLI install directory.
 */
export function resolveCliDir(version: string, providerId: string): string {
    return `${agentmuxHome()}/instances/v${version}/cli/${providerId}`;
}

/**
 * Resolve the effective provider for a launch, preferring the agent's
 * bound ABF bundle's copy over its own (driftable) `provider` field.
 *
 * The bundle is the readonly-once-set source of truth
 * (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.1);
 * `AgentDefinition.provider` can drift post-creation via `agent.define`'s
 * `if_exists=update` path. The backend already resolves this way for
 * both `agent.open`'s own spawn path (`agent_open.rs`) and the layer-3
 * credential gate (`identity/resolver/inject.rs`) — without this, the
 * CLI binary `launchAgentDefinition` actually launches could disagree
 * with which provider's credentials the backend gate validates and
 * injects (PR #2592 review — fixing only the backend gate wasn't
 * sufficient).
 *
 * Extracted as its own function, separate from `launchAgentDefinition`,
 * so this resolution logic is unit-testable in isolation — that
 * function's own RPC/side-effect surface (Node.js checks, CLI
 * resolution, content/skill loading, instance creation, etc.) has no
 * existing test harness anywhere in this codebase (every caller mocks
 * the whole function away), so testing this piece through it isn't
 * practical.
 *
 * Falls back to `agent.provider` on any failure (unbound, fetch error,
 * empty bundle provider) — this must never block a launch on its own.
 */
export async function resolveEffectiveLaunchProvider(agent: AgentDefinition): Promise<string> {
    if (!agent.memory_id) return agent.provider;
    try {
        const bundle = await RpcApi.GetMemoryCommand(TabRpcClient, { id: agent.memory_id });
        return bundle?.provider || agent.provider;
    } catch (e: any) {
        Logger.warn("agent", "Failed to resolve agent's bound bundle for provider; falling back to agent.provider", {
            agentId: agent.id,
            error: String(e),
        });
        return agent.provider;
    }
}
