// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /tools slash command — check and install AgentMux-managed CLI tools.
 *
 * Usage:
 *   /tools          → show tool status (same as /tools status)
 *   /tools status   → print status table to the chat
 *   /tools install  → install all missing tier-1 tools
 *   /tools install jq rg fd   → install specific tools by id
 */

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import type { SlashCommand, SlashCommandContext, SlashResult } from "../types";

// ── helpers ───────────────────────────────────────────────────────────────────

const STATUS_ICON: Record<string, string> = {
    installed_system:  "✓",
    installed_bundled: "✓",
    installed_managed: "✓",
    missing:           "✗",
    unavailable:       "—",
};

const STATUS_LABEL: Record<string, string> = {
    installed_system:  "system",
    installed_bundled: "bundled",
    installed_managed: "managed",
    missing:           "missing",
    unavailable:       "n/a on this platform",
};

function formatStatus(tools: ToolStatusEntry[]): string {
    if (tools.length === 0) return "No tools in catalog.";
    const lines = tools.map((t) => {
        const icon = STATUS_ICON[t.status] ?? "?";
        const label = STATUS_LABEL[t.status] ?? t.status;
        const ver = t.version ? ` ${t.version}` : "";
        const hint =
            t.status === "missing"
                ? `  — run /tools install ${t.id}`
                : "";
        return `  ${icon} ${t.display}${ver}  (${label})${hint}`;
    });
    return `Tool availability:\n\n${lines.join("\n")}`;
}

// ── handler ───────────────────────────────────────────────────────────────────

async function handleTools(ctx: SlashCommandContext, arg: string): Promise<SlashResult> {
    const parts = arg.trim().split(/\s+/).filter(Boolean);
    const sub = parts[0] ?? "status";

    // /tools  or  /tools status
    if (sub === "status" || sub === "") {
        try {
            const result = await RpcApi.GetToolStatusCommand(TabRpcClient, { timeout: 10000 });
            return { kind: "ok", message: formatStatus(result.tools) };
        } catch (err: any) {
            return { kind: "error", message: `failed to get tool status: ${err?.message ?? err}` };
        }
    }

    // /tools install [id...]
    if (sub === "install") {
        let toolIds: string[];

        if (parts.length > 1) {
            // Specific tools named
            toolIds = parts.slice(1);
        } else {
            // Install all missing tier-1 tools
            try {
                const status = await RpcApi.GetToolStatusCommand(TabRpcClient, { timeout: 10000 });
                toolIds = status.tools
                    .filter((t) => t.tier === 1 && t.status === "missing")
                    .map((t) => t.id);
            } catch (err: any) {
                return { kind: "error", message: `failed to check tool status: ${err?.message ?? err}` };
            }

            if (toolIds.length === 0) {
                return { kind: "ok", message: "All tier-1 tools are already installed." };
            }
        }

        try {
            const result = await RpcApi.InstallToolCommand(
                TabRpcClient,
                { tool_ids: toolIds },
                { timeout: 120000 }, // 2 min — downloads can be slow
            );

            const lines: string[] = [];
            for (const id of result.installed) {
                lines.push(`  ✓ ${id} installed`);
            }
            for (const f of result.failed) {
                lines.push(`  ✗ ${f.id}: ${f.error}`);
            }

            const kind = result.failed.length > 0 && result.installed.length === 0 ? "error" : "ok";
            return { kind, message: lines.join("\n") || "Done." };
        } catch (err: any) {
            return { kind: "error", message: `install failed: ${err?.message ?? err}` };
        }
    }

    return {
        kind: "error",
        message: `/tools: unknown subcommand '${sub}'. Use 'status' or 'install [id...]'.`,
    };
}

// ── export ────────────────────────────────────────────────────────────────────

export const toolsCommand: SlashCommand = {
    name: "tools",
    category: "system",
    description: "Check or install AgentMux-managed CLI tools (jq, rg, fd, …)",
    arg: { kind: "freeform", placeholder: "status | install [id...]", required: false },
    availability: "any-agent",
    handler: handleTools,
};
