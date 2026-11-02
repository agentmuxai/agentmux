// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent slug rules the window mirrors from `agentmux_common::slug`. The launch files themselves are rendered
 * by srv (`writeagentconfig` with `render_from_agent`), so these are all that is left of the window-side config
 * builder.
 */

/**
 * Derive a filesystem-safe slug from a display name. Lowercase, ASCII
 * alphanumeric + dash/underscore, consecutive dashes collapsed, trimmed to
 * 64 chars. Falls back to "agent" if the input has no valid characters.
 *
 * Mirrors `agentmux_common::slug::definition_slug` (crates/common/src/slug.rs),
 * or a SKILL.md preview built here won't match the path the Rust launch path
 * writes to. `slug-rules.test.ts` runs both over the Rust test table.
 */
export function deriveSlug(name: string): string {
    const filtered = name
        .toLowerCase()
        .replace(/[^a-z0-9-_]/g, "-");
    const collapsed = filtered
        .split("-")
        .filter((s) => s.length > 0)
        .join("-");
    const trimmed = collapsed.slice(0, 64);
    return trimmed || "agent";
}

/**
 * An agent's id when it has no slug: lowercase, every UTF-16 code unit outside
 * `[a-z0-9-_]` becomes `-`, no collapsing. Rust reproduces this as
 * `agentmux_common::slug::js_ascii_slug`, because signing-key names depend on
 * it; `slug-rules.test.ts` runs both over the Rust test table.
 */
export function jsAsciiSlug(name: string): string {
    return name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");
}
