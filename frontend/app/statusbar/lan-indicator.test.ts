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
});
