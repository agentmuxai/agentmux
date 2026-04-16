// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * buildStartupPayload — assembles the structured Markdown document sent as
 * the first user turn of a new agent session.
 *
 * Pure function: no RPC calls, no signals, no side effects. The caller
 * gathers inputs from existing reactive state and passes them in.
 *
 * See docs/specs/SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md
 */

import type { AccountProvider } from "@/app/view/identity/identity-model";

// ── Types ────────────────────────────────────────────────────────────────────

/** Hydrated account info safe for inclusion in the startup message. */
export interface ResolvedAccount {
    provider: string;
    name: string;
    kind: string;
    accessMethod: string;
    context: Record<string, string>;
}

export interface StartupPayloadOpts {
    agent: ForgeAgent;
    providerDisplayName: string;
    workDir: string;
    version: string;
    accounts: ResolvedAccount[];
    peerAgents: ForgeAgent[];
    startupContent: string | null;
}

/** Sentinel value in ForgeContent("startup") that disables the startup message. */
const SKIP_SENTINEL = "__SKIP__";

// ── Public API ───────────────────────────────────────────────────────────────

/**
 * Assemble the startup payload. Returns null if the agent has opted out
 * via the __SKIP__ sentinel.
 */
export function buildStartupPayload(opts: StartupPayloadOpts): string | null {
    if (opts.startupContent?.trim() === SKIP_SENTINEL) return null;

    const parts: string[] = [];
    const date = new Date().toISOString().slice(0, 10);

    // ── Identity ─────────────────────────────────────────────────────────
    parts.push("# Session Context\n");
    parts.push("## Identity\n");
    parts.push(`- **Name:** ${opts.agent.name}\n`);
    if (opts.agent.slug && opts.agent.slug !== opts.agent.name) {
        parts.push(`- **Slug:** ${opts.agent.slug}\n`);
    }
    parts.push(`- **Provider:** ${opts.providerDisplayName}\n`);
    if (opts.workDir) {
        parts.push(`- **Working Directory:** ${opts.workDir}\n`);
    }
    parts.push(`- **AgentMux Version:** ${opts.version}\n`);
    parts.push(`- **Date:** ${date}\n`);

    // ── Description ──────────────────────────────────────────────────────
    if (opts.agent.description?.trim()) {
        parts.push("\n## Description\n");
        parts.push(opts.agent.description.trim() + "\n");
    }

    // ── Assigned Accounts ────────────────────────────────────────────────
    if (opts.accounts.length > 0) {
        parts.push("\n## Assigned Accounts\n");
        for (const acct of opts.accounts) {
            parts.push(`\n### ${acct.provider} — ${acct.name}\n`);
            parts.push(`- **Kind:** ${acct.kind}\n`);
            parts.push(`- **Access:** ${acct.accessMethod}\n`);
            for (const [key, val] of Object.entries(acct.context)) {
                if (val) {
                    parts.push(`- **${formatContextKey(key)}:** ${val}\n`);
                }
            }
        }
    }

    // ── Custom Startup Instructions ──────────────────────────────────────
    if (opts.startupContent?.trim()) {
        const vars = buildTemplateVars(opts, date);
        const expanded = expandTemplate(opts.startupContent.trim(), vars);
        parts.push("\n## Startup Instructions\n");
        parts.push(expanded + "\n");
    }

    // ── Peer Agents ──────────────────────────────────────────────────────
    const peers = opts.peerAgents.filter((a) => a.id !== opts.agent.id);
    if (peers.length > 0) {
        parts.push("\n## Peer Agents\n");
        // Cap at 10 to keep payload reasonable
        const shown = peers.slice(0, 10);
        for (const peer of shown) {
            const desc = peer.description?.trim() ? ` — ${peer.description.trim()}` : "";
            parts.push(`- **${peer.name}** (${peer.provider})${desc}\n`);
        }
        if (peers.length > 10) {
            parts.push(`- ...and ${peers.length - 10} more\n`);
        }
    }

    // ── Action directive ─────────────────────────────────────────────────
    // Without an explicit instruction at the end, the agent treats the
    // payload as informational context and responds "Ready to help."
    if (opts.startupContent?.trim()) {
        parts.push("\n---\n");
        parts.push("**ACTION REQUIRED:** Execute the verification round above now. ");
        parts.push("Run each check, fix any failures, and report the status table before proceeding.\n");
    }

    return parts.join("");
}

// ── Account Resolution ───────────────────────────────────────────────────────

/**
 * Resolve a ForgeAgent's account assignments into hydrated ResolvedAccount
 * objects. Reads from the Identity localStorage store.
 *
 * Never includes secrets — only metadata and access method descriptors.
 */
export function resolveAccounts(
    agentAccounts: Partial<Record<AccountProvider, string | null>>,
    allAccounts: { id: string; name: string; provider: string; kind: string; secret_ref: any; context: any }[],
): ResolvedAccount[] {
    const resolved: ResolvedAccount[] = [];

    for (const [provider, accountId] of Object.entries(agentAccounts)) {
        if (!accountId) continue;
        const account = allAccounts.find((a) => a.id === accountId);
        if (!account) continue;

        resolved.push({
            provider,
            name: account.name,
            kind: account.kind,
            accessMethod: describeAccessMethod(account.secret_ref),
            context: flattenContext(account.context),
        });
    }

    return resolved;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function describeAccessMethod(ref: any): string {
    if (!ref) return "unknown";
    switch (ref.backend) {
        case "env":
            return ref.env_var ? `env:${ref.env_var}` : "env (unspecified)";
        case "secrets_manager":
            return ref.sm_path ? `secrets_manager:${ref.sm_path}` : "secrets_manager";
        case "plaintext_dev":
            return "plaintext (dev-only)";
        default:
            return ref.backend || "unknown";
    }
}

function flattenContext(ctx: any): Record<string, string> {
    if (!ctx || typeof ctx !== "object") return {};
    const flat: Record<string, string> = {};
    for (const [key, val] of Object.entries(ctx)) {
        if (val == null) continue;
        if (typeof val === "string" && val) {
            flat[key] = val;
        } else if (Array.isArray(val) && val.length > 0) {
            flat[key] = val.join(", ");
        }
    }
    return flat;
}

function formatContextKey(key: string): string {
    return key
        .replace(/_/g, " ")
        .replace(/\b\w/g, (c) => c.toUpperCase());
}

function buildTemplateVars(opts: StartupPayloadOpts, date: string): Record<string, string> {
    return {
        AGENT: opts.agent.name,
        AGENT_DISPLAY: opts.agent.name,
        AGENT_SLUG: opts.agent.slug || opts.agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-"),
        AGENT_ID: opts.agent.id,
        WORKING_DIR: opts.workDir,
        DATE: date,
        VERSION: opts.version,
        PROVIDER: opts.providerDisplayName,
    };
}

function expandTemplate(content: string, vars: Record<string, string>): string {
    return content.replace(/\{\{(\w+)\}\}/g, (match, key) => {
        return vars[key] ?? match;
    });
}
