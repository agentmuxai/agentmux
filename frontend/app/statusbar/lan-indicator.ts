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
 * "Off", "on but nobody out there", and "on but the daemon failed to start"
 * are different facts the user acts on differently, so each gets its own
 * state here.
 *
 * Pure so the state table can be tested without mounting the status bar.
 */
export type LanIndicatorState = "peers" | "idle" | "error" | "undiscoverable" | "off";

export interface LanIndicatorInput {
    /** The `network:lan_discovery` setting. */
    enabled: boolean;
    /** `lanDiscoverabilityAtom().state` — whether OTHER machines can find this
     *  one. `"undiscoverable"` means the mDNS service never announced on any
     *  IPv4 address, so peers cannot see us even though we may see them (Area54,
     *  2026-10-01). It outranks "peers": hearing peers says nothing about being
     *  heard, which is the whole failure. */
    discoverability?: "healthy" | "degraded" | "undiscoverable" | "off" | null;
    /** Number of DISCOVERED peers — excludes this instance, post-#3025. */
    peerCount: number;
    /** `lanDiscoveryErrorAtom` — set when the mDNS daemon could not be
     *  started (the Windows firewall "Block" path is the common one). The
     *  setting stays enabled in that case, so without this the failure would
     *  render as the muted, documented-as-healthy idle state [codex P2]. */
    error?: string | null;
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
    // Off wins outright: the user turned it off, so a stale error from a
    // previous attempt is not something to nag about.
    if (!input.enabled) {
        return { state: "off", glyph: "◇", label: "LAN discovery off — click to enable" };
    }
    // Invisible to everyone else beats "peers": reaching a peer proves we can
    // HEAR, not that anyone can hear us, and one-way is the failure this exists
    // to show. The glyph stays hollow: nobody can see this machine.
    if (input.discoverability === "undiscoverable") {
        return {
            state: "undiscoverable",
            glyph: "◇",
            label: "LAN: other machines can't see this one. Another program may be using the mDNS port (5353). Turn LAN off and on to retry",
        };
    }
    // Peers outrank an error, mirroring the popover, whose peers rows are NOT
    // gated on the error while its "no peers" row IS (`!lanDiscoveryError()`).
    // Reaching a peer is direct proof discovery works, which makes a lingering
    // error stale rather than current.
    //
    // Defensive `> 0`: a negative count is nonsense, but treating it as "peers
    // found" would be the worse failure — it would claim peers exist while
    // the popover lists none.
    if (input.peerCount > 0) {
        return { state: "peers", glyph: "◆", label: `${input.peerCount} on LAN` };
    }
    // Enabled, nothing found, and the daemon reported a failure: the reason
    // there are no peers is that discovery never started. Must not render as
    // the healthy idle state [codex P2].
    if (input.error) {
        return { state: "error", glyph: "◇", label: `LAN discovery failed: ${input.error}` };
    }
    return { state: "idle", glyph: "◇", label: "LAN discovery on — no peers found" };
}
