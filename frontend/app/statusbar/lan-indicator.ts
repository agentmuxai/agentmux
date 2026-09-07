// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which LAN indicator to show next to the hostname in the status bar.
 *
 * Three states, not two. The indicator used to render only when peers were
 * found (`lanCount() > 0`), which was correct per
 * `docs/specs/hostname-popover.md` — but until #3025 every instance discovered
 * ITSELF as a phantom peer, so the count was never 0 while discovery was on.
 * The glyph therefore behaved like an "enabled" lamp, and that is how it was
 * learned. #3025 removed the phantom, and a lone instance with discovery
 * genuinely enabled started rendering exactly like one with it switched off.
 * See docs/retro/retro-lan-diamond-vanished-after-self-peer-fix-2026-09-06.md.
 *
 * "Off" and "on but nobody out there" are different facts the user acts on
 * differently, so each gets its own state here.
 *
 * Pure so the state table can be tested without mounting the status bar —
 * the same extract-and-test pattern as `tab-strip-visibility.ts`.
 */
export type LanIndicatorState = "peers" | "idle" | "off";

export interface LanIndicatorInput {
    /** The `network:lan_discovery` setting. */
    enabled: boolean;
    /** Number of DISCOVERED peers — excludes this instance, post-#3025. */
    peerCount: number;
}

export interface LanIndicator {
    state: LanIndicatorState;
    /** Filled only when real peers exist, so the peers/no-peers distinction
     *  survives without relying on color. */
    glyph: "◆" | "◇";
    /** Reused for both the tooltip and the accessible name. */
    label: string;
}

export function resolveLanIndicator(input: LanIndicatorInput): LanIndicator {
    if (!input.enabled) {
        return { state: "off", glyph: "◇", label: "LAN discovery off — click to enable" };
    }
    // Defensive `<= 0`: a negative count is nonsense, but treating it as
    // "peers found" would be the worse failure — it would claim peers exist
    // while the popover lists none.
    if (input.peerCount <= 0) {
        return { state: "idle", glyph: "◇", label: "LAN discovery on — no peers found" };
    }
    return {
        state: "peers",
        glyph: "◆",
        label: `${input.peerCount} on LAN`,
    };
}
