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
export type LanIndicatorState =
    | "peers"
    | "idle"
    | "error"
    | "undiscoverable"
    | "blocked"
    | "needs-setup"
    | "public-network"
    | "managed"
    | "off";

export interface LanIndicatorInput {
    /** The `network:lan_discovery` setting. */
    enabled: boolean;
    /** `lanDiscoverabilityAtom().state` — whether OTHER machines can find this
     *  one. `"undiscoverable"` means the mDNS service never announced on any
     *  IPv4 address, so peers cannot see us even though we may see them (Area54,
     *  2026-10-01). It outranks "peers": hearing peers says nothing about being
     *  heard, which is the whole failure. */
    discoverability?: "healthy" | "degraded" | "undiscoverable" | "off" | null;
    /** `lanFirewallAtom().status` — what the OS firewall would do to a peer
     *  trying to reach this machine (Windows only for now; `null` elsewhere).
     *  `"blocked"` is a fact (an enabled Block rule matches us, and Windows
     *  enforces it over any Allow), so it outranks peers. The other three are
     *  inferred from the ABSENCE of an allow rule and can be wrong for unusual
     *  rules, so seeing peers outranks them: a peer proves discovery works.
     *  See SPEC_LAN_FIREWALL_SETUP_2026_10_01.md 4.3. */
    firewall?: "ok" | "needs-setup" | "blocked" | "public-network" | "managed" | "unknown" | "off" | null;
    /** Number of DISCOVERED peers — excludes this instance, post-#3025. */
    peerCount: number;
    /** `lanDiscoveryErrorAtom` — set when the mDNS daemon could not be
     *  started (the Windows firewall "Block" path is the common one). The
     *  setting stays enabled in that case, so without this the failure would
     *  render as the muted, documented-as-healthy idle state [codex P2]. */
    error?: string | null;
}

/** The one wording for each firewall problem, shared by the status-bar tooltip
 *  and the popover so the two cannot disagree. */
export const FIREWALL_MESSAGES = {
    blocked: "LAN: Windows Firewall is blocking AgentMux (a Block rule matches it). Other devices can't reach this one",
    "needs-setup":
        "LAN needs one-time setup: Windows Firewall has no rule letting other devices reach this AgentMux",
    "public-network":
        "LAN: Windows treats this network as Public, which blocks incoming connections. Mark it Private to use LAN",
    managed: "LAN: your administrator manages the firewall for this device, so AgentMux cannot open it",
} as const;

export function firewallMessage(status: string | null | undefined): string | null {
    return status != null && status in FIREWALL_MESSAGES ? FIREWALL_MESSAGES[status as keyof typeof FIREWALL_MESSAGES] : null;
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
    // A matching Block rule is a fact, not a guess: Windows drops inbound
    // traffic for us on that profile whatever else is allowed.
    if (input.firewall === "blocked") {
        return {
            state: "blocked",
            glyph: "◇",
            label: FIREWALL_MESSAGES.blocked,
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
    // No peers, and the firewall has no rule letting them in. This is the state
    // narko was in on 2026-09-30: LAN on, listeners up, mDNS registered, nothing
    // logged, no peer ever seen. Inferred from a missing rule, which is why
    // peers (above) outrank it.
    if (input.firewall === "needs-setup") {
        return {
            state: "needs-setup",
            glyph: "◇",
            label: FIREWALL_MESSAGES["needs-setup"],
        };
    }
    if (input.firewall === "public-network") {
        return {
            state: "public-network",
            glyph: "◇",
            label: FIREWALL_MESSAGES["public-network"],
        };
    }
    if (input.firewall === "managed") {
        return {
            state: "managed",
            glyph: "◇",
            label: FIREWALL_MESSAGES.managed,
        };
    }
    // Enabled, nothing found, and the daemon reported a failure: the reason
    // there are no peers is that discovery never started. Must not render as
    // the healthy idle state [codex P2].
    if (input.error) {
        return { state: "error", glyph: "◇", label: `LAN discovery failed: ${input.error}` };
    }
    return { state: "idle", glyph: "◇", label: "LAN discovery on — no peers found" };
}
