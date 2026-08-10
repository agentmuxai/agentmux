// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Per-agent zoom seeding — frontend counterpart of
// agentmux-srv/src/server/app_api/mod.rs::parse_seed_zoom. Extracted out
// of agent-model.ts::launchAgentDefinition (SPEC_AGENT_ZOOM_PERSISTENCE_
// 2026_06_22.md; the frontend-path gap is documented in
// SPEC_AGENT_COLOR_2026_08_08.md §3.5) so the clamp/validate contract has
// its own unit tests instead of living inline in a large function.

/**
 * Parse + validate a saved `ui:zoom` content blob for seeding a launched
 * agent block's `term:zoom`. Returns a parseable, non-default (≠ 1), in-
 * [0.5, 2] zoom, or `null` for anything else (missing, default, out of
 * range, garbage) — the range term.tsx's own zoom control enforces.
 *
 * `null` (not "omit the key") is the deliberate return value for "no seed
 * value" — `launchAgentDefinition` sets `term:zoom` unconditionally so
 * relaunching into a reused block (`targetBlockId`, the fork/relaunch
 * flows) can't leave a stale zoom from whatever agent previously occupied
 * that block (`SetMetaCommand` merges meta rather than replacing it —
 * reagent/codex P2, PR #2477). `null` is also term.tsx's own "reset to
 * default" sentinel (term.tsx:347), so it round-trips through the same
 * read path the drag-to-zoom handler already uses.
 */
export function parseSeedZoom(raw: string | null | undefined): number | null {
    const trimmed = raw?.trim();
    if (!trimmed) return null;
    const z = Number(trimmed);
    if (!Number.isFinite(z) || z === 1 || z < 0.5 || z > 2) return null;
    return z;
}
