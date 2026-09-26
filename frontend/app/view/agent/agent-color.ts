// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Per-agent color assignment — frontend counterpart of
// agentmux-srv/src/backend/agent_color.rs.
//
// Duplicated here (not shared) because agent launch has two independent
// meta-building paths — the backend's `agent.open` RPC
// (agent_open.rs::register_agent_open) and this frontend's
// `launchAgentDefinition` (agent-model.ts), which is what the picker's
// "Continue" click actually calls. `pickAgentColor` here is a same-shape
// FNV-1a hash for algorithmic consistency, but the two never need to
// agree bit-for-bit: whichever code path assigns a color FIRST persists
// it via SetAgentContentCommand/agent_content_set, and every later reader
// (either path) just reads that stored value back. See
// docs/specs/SPEC_AGENT_COLOR_2026_08_08.md.

// Deliberately NOT derived from tab.tsx's TAB_COLORS — that array was
// desaturated for the tab strip (docs/specs/SPEC_TAB_COLOR_DESATURATION_2026_08_13.md)
// and agent pane borders must keep the original vivid hues. Mirrors
// agentmux-srv/src/backend/agent_color.rs::AGENT_COLOR_PALETTE (same 14
// hexes, same order — keep both in sync if this list changes).
export const AGENT_COLOR_PALETTE: string[] = [
    "#ef4444", // Red
    "#f97316", // Orange
    "#f59e0b", // Amber
    "#eab308", // Yellow
    "#84cc16", // Lime
    "#22c55e", // Green
    "#14b8a6", // Teal
    "#06b6d4", // Cyan
    "#3b82f6", // Blue
    "#6366f1", // Indigo
    "#8b5cf6", // Violet
    "#d946ef", // Fuchsia
    "#ec4899", // Pink
    "#f43f5e", // Rose
];

/** Deterministic FNV-1a hash of the agent id onto the palette — stable
 * across reloads/machines, no RNG needed. Mirrors
 * agent_color.rs::pick_agent_color. */
export function pickAgentColor(agentId: string): string {
    let hash = BigInt("0xcbf29ce484222325");
    const fnvPrime = BigInt("0x100000001b3");
    const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
    for (let i = 0; i < agentId.length; i++) {
        hash ^= BigInt(agentId.charCodeAt(i) & 0xff);
        hash = (hash * fnvPrime) & mask;
    }
    const idx = Number(hash % BigInt(AGENT_COLOR_PALETTE.length));
    return AGENT_COLOR_PALETTE[idx];
}

/** Strict `#rrggbb` shape check — same contract as the Rust counterpart:
 * the value flows into block meta applied as a CSS border-color. */
export function isValidAgentColor(value: string | null | undefined): value is string {
    return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value);
}

/** Dimmed variant for the unfocused pane border (`frame:bordercolor`) —
 * full-strength color stays on the focused border
 * (`frame:activebordercolor`). Mirrors agent_color.rs::dim_agent_color. */
export function dimAgentColor(hex: string): string {
    if (!isValidAgentColor(hex)) return hex;
    const channel = (s: string) => parseInt(s, 16);
    const r = Math.round(channel(hex.slice(1, 3)) * 0.55);
    const g = Math.round(channel(hex.slice(3, 5)) * 0.55);
    const b = Math.round(channel(hex.slice(5, 7)) * 0.55);
    const hex2 = (n: number) => n.toString(16).padStart(2, "0");
    return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}
