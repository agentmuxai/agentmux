// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Launch-environment helpers extracted from agent-model.ts (see
 * docs/specs — modularization pass, 2026-07-23). These resolve
 * host-level paths/availability needed before spawning an agent CLI.
 * No `this`/class coupling — standalone functions the model calls into.
 */

import { getApi } from "@/app/store/global";
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
