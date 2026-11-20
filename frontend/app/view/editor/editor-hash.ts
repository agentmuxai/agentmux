// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Hash a string with SHA-256 → hex. Used for the slice's contentHash field
 *  so the saga (Phase 1C) can decide whether a tab is dirty vs. disk. Cheap;
 *  files are capped at 10 MB by the read RPC. */
export async function sha256Hex(s: string): Promise<string> {
    const buf = new TextEncoder().encode(s);
    const digest = await crypto.subtle.digest("SHA-256", buf);
    return Array.from(new Uint8Array(digest))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
}
