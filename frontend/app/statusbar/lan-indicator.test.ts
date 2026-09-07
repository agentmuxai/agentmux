// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveLanIndicator } from "./lan-indicator";

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
});
