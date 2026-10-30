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
 * - `agentmux_container_start` — a container agent's container is coming up:
 *   "Downloading the agent image… 42%", "Starting the container…", then
 *   "Container ready (12 s)" or "Couldn't start the container". Every frame of
 *   one start-up shares a `start_id`, so it is one row that updates in place.
 * - `agentmux_agent_ready` — a container agent is up and nothing is on its way to it: "Ready: send a
 *   message to start", so the pane never ends a start-up silent.
 * - `agentmux_container_image` — the container runs an older agent image than this AgentMux:
 *   "This container uses the 0.59.41 image" with an Update container button, then (one row, updated
 *   in place) "Container removed. The next message starts it from the 0.60.2 image", or, when AgentMux moved
 *   an idle agent's container itself, "Updated the container to the 0.60.2 image".
 * - `agentmux_container_limit` — a container's CPU or memory limit changed ("Memory limit set to 16 GB by Agent1"), or
 *   a process in it was stopped for going over its memory limit ("A process in this container was stopped: it ran out
 *   of its 8 GB memory limit") with a "Raise the limit" button. Each frame is its own row.
 * - `agentmux_mcp_health` — the agent's AgentMux MCP server didn't connect and
 *   AgentMux is repairing it (`crates/srv/src/backend/mcp_health.rs`). Every
 *   frame of one episode shares an `episode`, so it is one row that updates.
 *
 * Shared by the live path (`useAgentStream.ts`) and replay
 * (`parseHistoryLines.ts`), like `session-outcome.ts`. The provider CLI never
 * emits these, so both intercept them before the translator.
 */

import { PROVIDERS } from "./providers/catalog";
import { cpuText, memoryText } from "./container-limits";
import type { CliNoticeNode } from "./types";

const CLI_INSTALL_SUBTYPE = "agentmux_cli_install";
const CLI_VERSION_CHANGED_SUBTYPE = "agentmux_cli_version_changed";
const MCP_HEALTH_SUBTYPE = "agentmux_mcp_health";
const CONTAINER_START_SUBTYPE = "agentmux_container_start";
const CONTAINER_IMAGE_SUBTYPE = "agentmux_container_image";
const CONTAINER_LIMIT_SUBTYPE = "agentmux_container_limit";
const AGENT_READY_SUBTYPE = "agentmux_agent_ready";

const MCP_STATES = ["healing", "restarting", "ok", "gave_up"] as const;
const CONTAINER_START_STATES = ["pulling", "starting", "ready", "failed"] as const;

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
        // Claude Code's own frame names no provider; srv's for a Codex pane does.
        provider: str(e.provider) ?? "claude",
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
    if (e.subtype === CONTAINER_START_SUBTYPE) {
        const startId = str(e.start_id);
        const startState = CONTAINER_START_STATES.find((s) => s === e.state);
        if (!startId || !startState) return null;
        return {
            type: "cli_notice",
            id: `container-start-${startId}`,
            kind: "container_start",
            provider,
            startState,
            percent: typeof e.percent === "number" ? e.percent : undefined,
            image: str(e.image) ?? undefined,
            seconds: typeof e.seconds === "number" ? e.seconds : undefined,
            error: str(e.error) ?? undefined,
            timestamp,
        };
    }
    if (e.subtype === AGENT_READY_SUBTYPE) {
        const readyId = str(e.ready_id);
        if (!readyId) return null;
        return {
            type: "cli_notice",
            id: `agent-ready-${readyId}`,
            kind: "agent_ready",
            provider,
            seconds: typeof e.seconds === "number" ? e.seconds : undefined,
            timestamp,
        };
    }
    if (e.subtype === CONTAINER_IMAGE_SUBTYPE) {
        const imageState = e.state === "outdated" || e.state === "updated" || e.state === "auto_updated" ? e.state : null;
        const blockId = str(e.block_id);
        const current = str(e.current);
        if (!imageState || !blockId || !current) return null;
        return {
            type: "cli_notice",
            id: `container-image-${blockId}`,
            kind: "container_image",
            provider,
            imageState,
            running: str(e.running) ?? undefined,
            current,
            blockId,
            timestamp,
        };
    }
    if (e.subtype === CONTAINER_LIMIT_SUBTYPE) {
        const limitState = e.state === "changed" || e.state === "oom" || e.state === "failed" ? e.state : null;
        const eventId = str(e.event_id);
        if (!limitState || !eventId) return null;
        return {
            type: "cli_notice",
            id: `container-limit-${eventId}`,
            kind: "container_limit",
            provider,
            limitState,
            memoryMb: typeof e.memory_mb === "number" ? e.memory_mb : undefined,
            cpus: typeof e.cpus === "number" ? e.cpus : undefined,
            by: str(e.by) ?? undefined,
            error: str(e.error) ?? undefined,
            blockId: str(e.block_id) ?? undefined,
            agentId: str(e.agent_id) ?? undefined,
            agentName: str(e.agent_name) ?? undefined,
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
export function cliNoticeText(node: CliNoticeNode, appVersion?: string): { label: string; detail: string | null; tone: CliNoticeTone } {
    const name = cliDisplayName(node.provider);
    if (node.kind === "compaction_failed") {
        return { label: `${name} couldn't compact the conversation`, detail: node.error ?? null, tone: "error" };
    }
    if (node.kind === "mcp_health") return mcpHealthText(node);
    if (node.kind === "container_start") return containerStartText(node);
    if (node.kind === "container_image") return containerImageText(node, appVersion);
    if (node.kind === "container_limit") return containerLimitText(node);
    if (node.kind === "agent_ready") {
        const took = node.seconds != null ? ` (${Math.max(1, Math.round(node.seconds))} s)` : "";
        return { label: `Ready${took}: send a message to start`, detail: `${name} is installed and the container is running`, tone: "info" };
    }
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

/** The container's agent image against this AgentMux's, one row per pane. */
function containerImageText(node: CliNoticeNode, appVersion?: string): { label: string; detail: string | null; tone: CliNoticeTone } {
    if (node.imageState === "auto_updated") {
        return {
            label: `Updated the container to the ${node.current} image`,
            detail: `It was on the ${node.running ?? "older"} image. Tools installed inside it are installed again; the workspace and memory are kept`,
            tone: "info",
        };
    }
    if (node.imageState === "updated") {
        return {
            label: "Container removed",
            detail: `The next message starts it from the ${node.current} image`,
            tone: "info",
        };
    }
    return {
        label: `This container uses the ${node.running ?? "older"} image`,
        // The row is kept in the conversation; the AgentMux that wrote it may be an older one than the one showing it.
        detail: `This AgentMux runs ${appVersion || node.current}. Updating resets tools installed inside the container; the workspace and memory are kept`,
        tone: "warn",
    };
}

/** A container's CPU and memory limits: a change, or a process stopped for going over memory. One row per event. */
function containerLimitText(node: CliNoticeNode): { label: string; detail: string | null; tone: CliNoticeTone } {
    const memory = node.memoryMb != null ? memoryText(node.memoryMb) : null;
    if (node.limitState === "oom") {
        return {
            label: memory
                ? `A process in this container was stopped: it ran out of its ${memory} memory limit`
                : "A process in this container was stopped: it ran out of memory",
            detail: memory
                ? "Reduce what runs at once (for a build, fewer parallel jobs), or raise the limit"
                : "Docker's own memory may be too small: raise it in Docker Desktop's resource settings, or reduce what runs at once",
            tone: "warn",
        };
    }
    if (node.limitState === "failed") {
        return { label: "Docker did not take the new container limits", detail: node.error ?? null, tone: "error" };
    }
    const parts = [
        memory ? `Memory limit ${memory}` : "Memory unlimited",
        node.cpus != null ? cpuText(node.cpus) : "CPUs unlimited",
    ];
    return { label: `Container limits: ${parts.join(", ")}${node.by ? `, set by ${node.by}` : ""}`, detail: null, tone: "info" };
}

/** A container agent's start-up, one row per start. */
function containerStartText(node: CliNoticeNode): { label: string; detail: string | null; tone: CliNoticeTone } {
    switch (node.startState) {
        case "pulling": {
            const percent = node.percent != null ? ` ${Math.round(node.percent)}%` : "";
            return {
                label: `Downloading the agent image…${percent}`,
                detail: "First start only. The agent answers once it's downloaded",
                tone: "info",
            };
        }
        case "starting":
            return { label: "Starting the container…", detail: null, tone: "info" };
        case "ready": {
            const took = node.seconds != null ? ` (${Math.max(1, Math.round(node.seconds))} s)` : "";
            return { label: `Container ready${took}`, detail: null, tone: "info" };
        }
        default:
            return { label: "Couldn't start the container", detail: node.error ?? null, tone: "error" };
    }
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
