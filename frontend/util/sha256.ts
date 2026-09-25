// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** SHA-256 of a string's UTF-8 bytes, as lowercase hex — the same digest the
 *  backend computes for memory content (`content_hash` in
 *  agent_native_memory_versions.rs / bundle_versions.rs), so a hash taken
 *  here can be sent back as a `base_sha256` and compared server-side. Same
 *  implementation as editor-model.ts's private helper. */
export async function sha256Hex(s: string): Promise<string> {
    const buf = new TextEncoder().encode(s);
    const digest = await crypto.subtle.digest("SHA-256", buf);
    return Array.from(new Uint8Array(digest))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
}
