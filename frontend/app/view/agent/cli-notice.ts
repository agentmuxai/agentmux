// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane rows for an agent's CLI, from srv's own `system` frames
 * (`crates/srv/src/backend/cli_notice.rs`;
 * SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §6.3):
 *
 * - `agentmux_cli_install` — "Installing Claude Code 2.1.285…", then
 *   "Installed … (14 s)" or "Couldn't install …". Every frame of one install
 *   shares an `install_id`, so it is one row that updates in place.
 * - `agentmux_cli_version_changed` — "Claude Code updated: 2.1.218 →
 *   2.1.285", with a warning when the running version isn't the one AgentMux
 *   pins.
 * - `agentmux_mcp_health` — the agent's AgentMux MCP server didn't connect and
 *   AgentMux is repairing it (`crates/srv/src/backend/mcp_health.rs`). Every
 *   frame of one episode shares an `episode`, so it is one row that updates.
 *
 * Shared by the live path (`useAgentStream.ts`) and replay
 * (`parseHistoryLines.ts`), like `session-outcome.ts`. The provider CLI never
 * emits these, so both intercept them before the translator.
 */

import { PROVIDERS } from "./providers/catalog";
import type { CliNoticeNode } from "./types";

const CLI_INSTALL_SUBTYPE = "agentmux_cli_install";
const CLI_VERSION_CHANGED_SUBTYPE = "agentmux_cli_version_changed";
const MCP_HEALTH_SUBTYPE = "agentmux_mcp_health";

const MCP_STATES = ["healing", "restarting", "ok", "gave_up"] as const;

function str(v: unknown): string | null {
    return typeof v === "string" && v.length > 0 ? v : null;
}

function frameTime(v: unknown): number | null {
    const parsed = typeof v === "string" ? Date.parse(v) : NaN;
    return Number.isNaN(parsed) ? null : parsed;
}

/** The CLI's own prefix on a failed compaction's reason; the row says it already. */
const COMPACTION_ERROR_PREFIX = "Error during compaction: ";

/**
 * A failed compaction: the CLI's `system/status` frame with
 * `compact_result: "failed"` (and `compact_error`). It is the only trace of
 * the failure — the turn then ends with `is_error: false` — so without a row
 * the user sees nothing. Other status frames (the `compacting` start and its
 * ~30 s heartbeat, a success) are not this. The frame's `uuid` makes the id
 * stable live and on replay; a frame without one is skipped.
 */
function compactionFailedNode(e: Record<string, unknown>, now: number): CliNoticeNode | null {
    if (e.compact_result !== "failed") return null;
    const uuid = str(e.uuid);
    if (!uuid) return null;
    const raw = str(e.compact_error);
    const error = raw?.startsWith(COMPACTION_ERROR_PREFIX) ? raw.slice(COMPACTION_ERROR_PREFIX.length) : raw;
    return {
        type: "cli_notice",
        id: `compaction-failed-${uuid}`,
        kind: "compaction_failed",
        provider: "claude",
        error: error ?? undefined,
        timestamp: frameTime(e.timestamp) ?? now,
    };
}

/**
 * The node for a CLI notice frame, or `null` if `rawEvent` isn't one or is
 * malformed. `now` stands in for a frame with no usable timestamp (the live
 * path passes `Date.now()`, replay passes the line's own time or 0).
 */
export function parseCliNoticeFrame(rawEvent: unknown, now: number): CliNoticeNode | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const e = rawEvent as Record<string, unknown>;
    if (e.type !== "system") return null;
    if (e.subtype === "status") return compactionFailedNode(e, now);
    const provider = str(e.provider);
    if (!provider) return null;
    const timestamp = frameTime(e.timestamp) ?? now;

    if (e.subtype === CLI_INSTALL_SUBTYPE) {
        const installId = str(e.install_id);
        const version = str(e.version);
        const state = e.state === "installing" || e.state === "installed" || e.state === "failed" ? e.state : null;
        if (!installId || !version || !state) return null;
        return {
            type: "cli_notice",
            id: `cli-install-${installId}`,
            kind: "install",
            provider,
            version,
            state,
            seconds: typeof e.seconds === "number" ? e.seconds : undefined,
            error: str(e.error) ?? undefined,
            timestamp,
        };
    }
    if (e.subtype === MCP_HEALTH_SUBTYPE) {
        const episode = str(e.episode);
        const mcpState = MCP_STATES.find((s) => s === e.state);
        if (!episode || !mcpState) return null;
        return {
            type: "cli_notice",
            id: `mcp-health-${episode}`,
            kind: "mcp_health",
            provider,
            mcpState,
            step: typeof e.step === "number" ? e.step : undefined,
            error: str(e.error) ?? undefined,
            safeProfile: e.safe_profile === true,
            timestamp,
        };
    }
    if (e.subtype === CLI_VERSION_CHANGED_SUBTYPE) {
        const from = str(e.from);
        const to = str(e.to);
        if (!from || !to) return null;
        return {
            type: "cli_notice",
            id: `cli-version-${provider}-${from}-${to}-${str(e.timestamp) ?? "notime"}`,
            kind: "version_changed",
            provider,
            from,
            to,
            pinned: str(e.pinned),
            timestamp,
        };
    }
    return null;
}

/** "Claude Code" for `claude`; the id itself for a provider the catalog doesn't know. */
function cliDisplayName(provider: string): string {
    return PROVIDERS[provider]?.displayName ?? provider;
}

export type CliNoticeTone = "info" | "warn" | "error";

/** What the row says: a short label, an optional detail line, and its tone. */
export function cliNoticeText(node: CliNoticeNode): { label: string; detail: string | null; tone: CliNoticeTone } {
    const name = cliDisplayName(node.provider);
    if (node.kind === "compaction_failed") {
        return { label: `${name} couldn't compact the conversation`, detail: node.error ?? null, tone: "error" };
    }
    if (node.kind === "mcp_health") return mcpHealthText(node);
    if (node.kind === "install") {
        switch (node.state) {
            case "installing":
                return { label: `Installing ${name} ${node.version}…`, detail: "The agent starts once it's installed", tone: "info" };
            case "installed": {
                const took = node.seconds != null ? ` (${Math.max(1, Math.round(node.seconds))} s)` : "";
                return { label: `Installed ${name} ${node.version}${took}`, detail: null, tone: "info" };
            }
            default:
                return { label: `Couldn't install ${name} ${node.version}`, detail: node.error ?? null, tone: "error" };
        }
    }
    const offPin = node.pinned != null && node.pinned !== node.to;
    return {
        label: `${name} updated: ${node.from} → ${node.to}`,
        detail: offPin ? `Not the version AgentMux pins (${node.pinned})` : null,
        tone: offPin ? "warn" : "info",
    };
}

const SAFE_PROFILE_DETAIL = "Running without the shell prefix until AgentMux restarts";

/** The repair of the agent's AgentMux MCP server, one row per episode. */
function mcpHealthText(node: CliNoticeNode): { label: string; detail: string | null; tone: CliNoticeTone } {
    const error = node.error ?? null;
    switch (node.mcpState) {
        case "healing":
            return {
                label: `AgentMux tools didn't connect. Reconnecting (try ${node.step ?? 1} of 2)…`,
                detail: error,
                tone: "warn",
            };
        case "restarting":
            return {
                label: "AgentMux tools still aren't connected. Restarting the agent without the shell prefix…",
                detail: error,
                tone: "warn",
            };
        case "ok":
            return {
                label: "AgentMux tools are connected again",
                detail: node.safeProfile ? SAFE_PROFILE_DETAIL : null,
                tone: "info",
            };
        default:
            return {
                label: "AgentMux tools aren't connected, and AgentMux couldn't repair them",
                detail: [error, "The agent was told to run `agentmux-mcp doctor`."].filter(Boolean).join(". "),
                tone: "error",
            };
    }
}
