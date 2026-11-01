// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Per-agent color assignment: the frontend counterpart of
// crates/srv/src/backend/agent_color.rs. The colours are the shared palette
// (theme/palette.ts, schema/color-palette.json), read by both sides, so
// there is no list to keep in sync.
//
// Agent launch has two independent meta-building paths: the backend's
// `agent.open` RPC (agent_open.rs::register_agent_open) and this frontend's
// `launchAgentDefinition` (agent-model.ts). Whichever assigns a colour FIRST
// persists it via SetAgentContentCommand/agent_content_set, and every later
// reader just reads that stored value back. See
// docs/specs/SPEC_AGENT_COLOR_2026_08_08.md and
// docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md.

import { pickSlot } from "@/app/theme/palette";

/** An agent's palette colour (`#rrggbb`): the plain FNV-1a hash of its id over
 * the shared palette. srv spreads new agents over the least-used slots
 * (palette.rs `pick_slot`); this is the fallback for a path that has no list
 * of the other agents' colours, and what a surface draws for an agent whose
 * colour is not stored yet. */
export function pickAgentColor(agentId: string): string {
    return pickSlot(agentId).hex;
}

/** Strict `#rrggbb` shape check: same contract as the Rust counterpart, since
 * the value flows into block meta applied as a CSS border-color. */
export function isValidAgentColor(value: string | null | undefined): value is string {
    return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value);
}
