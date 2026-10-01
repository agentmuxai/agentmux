// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { FIREWALL_MESSAGES, firewallMessage, resolveLanIndicator, shouldShowFirewallWarning } from "./lan-indicator";

describe("resolveLanIndicator", () => {
    it("shows the accent-filled diamond when real peers exist", () => {
        const r = resolveLanIndicator({ enabled: true, peerCount: 2 });
        expect(r.state).toBe("peers");
        expect(r.glyph).toBe("◆");
        expect(r.label).toBe("2 on LAN");
    });

    // The state that had NO representation at all before this change: #3025
    // stopped the phantom self-peer, so an enabled-but-alone instance fell to
    // peerCount 0 and rendered nothing — identical to discovery being off.
    it("distinguishes enabled-but-no-peers from off", () => {
        const idle = resolveLanIndicator({ enabled: true, peerCount: 0 });
        const off = resolveLanIndicator({ enabled: false, peerCount: 0 });

        expect(idle.state).toBe("idle");
        expect(off.state).toBe("off");
        expect(idle.state).not.toBe(off.state);
        expect(idle.label).not.toBe(off.label);
    });

    it("reports off regardless of a stale peer count", () => {
        // Peers linger in the atom for a tick after the setting is toggled
        // off; the setting is authoritative, not the leftover list.
        const r = resolveLanIndicator({ enabled: false, peerCount: 3 });
        expect(r.state).toBe("off");
        expect(r.glyph).toBe("◇");
    });

    it("fills the glyph only when peers exist, so state is not color-only", () => {
        expect(resolveLanIndicator({ enabled: true, peerCount: 1 }).glyph).toBe("◆");
        expect(resolveLanIndicator({ enabled: true, peerCount: 0 }).glyph).toBe("◇");
        expect(resolveLanIndicator({ enabled: false, peerCount: 0 }).glyph).toBe("◇");
    });

    it("treats a negative count as no peers rather than as peers", () => {
        const r = resolveLanIndicator({ enabled: true, peerCount: -1 });
        expect(r.state).toBe("idle");
    });

    // codex P2: the daemon failing to start leaves the SETTING enabled and
    // populates the error atom. Reporting that as the muted "idle" state hid a
    // real failure behind a tooltip documented as healthy.
    it("surfaces a start-up failure instead of the healthy idle state", () => {
        const r = resolveLanIndicator({
            enabled: true,
            peerCount: 0,
            error: "mDNS daemon failed to bind",
        });
        expect(r.state).toBe("error");
        expect(r.state).not.toBe("idle");
        expect(r.label).toContain("mDNS daemon failed to bind");
    });

    it("still reports off when disabled, even with a stale error", () => {
        const r = resolveLanIndicator({ enabled: false, peerCount: 0, error: "boom" });
        expect(r.state).toBe("off");
    });

    // Mirrors the popover, whose peer rows are not gated on the error while
    // its "no peers" row is: an actual peer proves discovery works.
    it("prefers peers over a lingering error", () => {
        const r = resolveLanIndicator({ enabled: true, peerCount: 2, error: "stale" });
        expect(r.state).toBe("peers");
        expect(r.label).toBe("2 on LAN");
    });

    it("treats null/undefined error as no error", () => {
        expect(resolveLanIndicator({ enabled: true, peerCount: 0, error: null }).state).toBe("idle");
        expect(resolveLanIndicator({ enabled: true, peerCount: 0 }).state).toBe("idle");
    });

    // Area54, 2026-10-01: it listed three peers while none of them listed it,
    // because its mDNS service never announced on an IPv4 interface. Reaching a
    // peer proves we can HEAR; it says nothing about being heard.
    describe("undiscoverable", () => {
        it("outranks peers: seeing others does not mean they see us", () => {
            const r = resolveLanIndicator({ enabled: true, peerCount: 3, discoverability: "undiscoverable" });
            expect(r.state).toBe("undiscoverable");
            expect(r.glyph).toBe("◇");
            expect(r.label).toContain("can't see this one");
            expect(r.label).toContain("5353");
        });

        it("is not confused with idle or error when nothing else is wrong", () => {
            const r = resolveLanIndicator({ enabled: true, peerCount: 0, discoverability: "undiscoverable" });
            expect(r.state).toBe("undiscoverable");
            expect(r.state).not.toBe("idle");
            expect(r.state).not.toBe("error");
        });

        it("still reports off when LAN is switched off, even with a stale verdict", () => {
            const r = resolveLanIndicator({ enabled: false, peerCount: 0, discoverability: "undiscoverable" });
            expect(r.state).toBe("off");
        });

        it("ignores healthy, degraded, off and null verdicts", () => {
            for (const d of ["healthy", "degraded", "off", null, undefined] as const) {
                expect(resolveLanIndicator({ enabled: true, peerCount: 2, discoverability: d }).state).toBe("peers");
                expect(resolveLanIndicator({ enabled: true, peerCount: 0, discoverability: d }).state).toBe("idle");
            }
        });
    });

    // SPEC_LAN_FIREWALL_SETUP_2026_10_01.md 4.3. On 2026-09-30 narko had LAN on,
    // listeners up, mDNS registered and nothing logged, because Windows had no
    // inbound rule for it: it saw no peer and said nothing.
    describe("firewall", () => {
        it("says needs-setup when no peers can be found and no rule lets them in", () => {
            const r = resolveLanIndicator({ enabled: true, peerCount: 0, firewall: "needs-setup" });
            expect(r.state).toBe("needs-setup");
            expect(r.glyph).toBe("◇");
            expect(r.label).toBe(FIREWALL_MESSAGES["needs-setup"]);
        });

        it("ranks a block above peers (a fact) but peers above inferred problems", () => {
            expect(resolveLanIndicator({ enabled: true, peerCount: 3, firewall: "blocked" }).state).toBe("blocked");
            for (const f of ["needs-setup", "public-network", "managed"] as const) {
                // A peer proves discovery works; a missing-rule inference can be wrong.
                expect(resolveLanIndicator({ enabled: true, peerCount: 3, firewall: f }).state).toBe("peers");
                expect(resolveLanIndicator({ enabled: true, peerCount: 0, firewall: f }).state).toBe(f);
            }
        });

        it("ranks undiscoverable above a firewall block, and off above everything", () => {
            expect(
                resolveLanIndicator({ enabled: true, peerCount: 0, discoverability: "undiscoverable", firewall: "blocked" })
                    .state,
            ).toBe("undiscoverable");
            for (const f of ["blocked", "needs-setup", "public-network", "managed"] as const) {
                expect(resolveLanIndicator({ enabled: false, peerCount: 0, firewall: f }).state).toBe("off");
            }
        });

        it("outranks a start-up error and the idle state", () => {
            const r = resolveLanIndicator({ enabled: true, peerCount: 0, error: "bind failed", firewall: "needs-setup" });
            expect(r.state).toBe("needs-setup");
            expect(resolveLanIndicator({ enabled: true, peerCount: 0, firewall: "ok" }).state).toBe("idle");
        });

        it("ignores ok, unknown, off and no verdict", () => {
            for (const f of ["ok", "unknown", "off", null, undefined] as const) {
                expect(resolveLanIndicator({ enabled: true, peerCount: 2, firewall: f }).state).toBe("peers");
                expect(resolveLanIndicator({ enabled: true, peerCount: 0, firewall: f }).state).toBe("idle");
            }
        });

        // ReAgent P2 on #4151: the popover must not warn while the bar shows peers.
        it("shows the popover warning on the same terms as the indicator", () => {
            for (const f of ["needs-setup", "public-network", "managed"] as const) {
                expect(shouldShowFirewallWarning(f, 0)).toBe(true);
                expect(shouldShowFirewallWarning(f, 2)).toBe(false);
            }
            // A block is a fact: it shows even with peers listed.
            expect(shouldShowFirewallWarning("blocked", 0)).toBe(true);
            expect(shouldShowFirewallWarning("blocked", 3)).toBe(true);
            for (const f of ["ok", "unknown", "off", null, undefined]) {
                expect(shouldShowFirewallWarning(f, 0)).toBe(false);
            }
            // Same inputs, same answer as the indicator.
            for (const peers of [0, 2]) {
                for (const f of ["blocked", "needs-setup", "public-network", "managed"] as const) {
                    const flagged = resolveLanIndicator({ enabled: true, peerCount: peers, firewall: f }).state === f;
                    expect(shouldShowFirewallWarning(f, peers)).toBe(flagged);
                }
            }
        });

        it("has one wording shared by the tooltip and the popover", () => {
            for (const f of ["blocked", "needs-setup", "public-network", "managed"] as const) {
                expect(firewallMessage(f)).toBe(FIREWALL_MESSAGES[f]);
                expect(resolveLanIndicator({ enabled: true, peerCount: 0, firewall: f }).label).toBe(firewallMessage(f));
            }
            for (const f of ["ok", "unknown", "off", null, undefined, "nonsense"]) {
                expect(firewallMessage(f)).toBeNull();
            }
        });
    });
});
