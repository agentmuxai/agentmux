// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Creating a file in an agent's native memory folder: the filename rules and
 * the one write, shared by Memory → Personal and the agent pane's own memory
 * view, so both check a name the way srv does and write it the same way.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** Validate a filename the same way the backend does, so the user gets
 *  feedback before the RPC round-trips. Mirrors validate_filename() in
 *  native_memory_handlers.rs: alphanumeric + `-_`, ends in `.md`, no path
 *  separators, stem ≤ 200 chars. */
export function validateMemoryFilename(filename: string): string | null {
    if (!filename) return "Filename must not be empty.";
    if (!filename.endsWith(".md")) return "Filename must end with .md.";
    if (filename.includes("/") || filename.includes("\\") || filename.includes("..")) {
        return "Filename must not contain path separators.";
    }
    const stem = filename.slice(0, -3);
    if (!stem) return "Filename needs a name before .md.";
    if (stem.length > 200) return "Filename is too long (max 200 chars).";
    if (!/^[a-zA-Z0-9_-]+$/.test(stem)) {
        return "Filename may only use letters, numbers, '-' and '_'.";
    }
    return null;
}

/** Normalize raw user input into a valid `.md` filename: trim, drop a
 *  leading path if pasted, append `.md` if omitted. */
export function normalizeMemoryFilename(raw: string): string {
    let name = raw.trim();
    if (!name) return name;
    // Strip any pasted directory portion.
    const slash = Math.max(name.lastIndexOf("/"), name.lastIndexOf("\\"));
    if (slash >= 0) name = name.slice(slash + 1);
    if (!name.toLowerCase().endsWith(".md")) name = `${name}.md`;
    return name;
}

/** Starter content for a freshly-created MEMORY.md index. */
export const MEMORY_MD_TEMPLATE = `# Memory Index

This file is loaded into every new Claude session for this agent.
Keep it to an index of topic files — detail lives in the topic files,
which Claude loads on demand.
`;

/**
 * Normalizes and checks `rawFilename`, then writes it as a human-authored file.
 * Resolves to the filename on success, or an error message to show: an invalid
 * name, a name `existing` already holds, or the write failing.
 */
export async function createMemoryFile(
    agentId: string,
    rawFilename: string,
    content: string,
    existing: readonly string[]
): Promise<{ filename: string } | { error: string }> {
    const filename = normalizeMemoryFilename(rawFilename);
    const invalid = validateMemoryFilename(filename);
    if (invalid) return { error: invalid };
    if (existing.includes(filename)) return { error: `${filename} already exists.` };
    try {
        await RpcApi.NativeMemoryWriteFileCommand(TabRpcClient, {
            agent_id: agentId,
            filename,
            content,
            provenance: { source: "human" },
        });
        return { filename };
    } catch (e) {
        return { error: `Create failed: ${(e as Error).message ?? e}` };
    }
}
