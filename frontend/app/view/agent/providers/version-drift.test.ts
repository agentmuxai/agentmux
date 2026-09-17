// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { compareVersions, isActionable, normalizeVersion, resolveDrift } from "./version-drift";

describe("normalizeVersion", () => {
    it("strips a leading v — node reports v22.22.2, gemini reports 0.60.0", () => {
        expect(normalizeVersion("v22.22.2")).toBe("22.22.2");
        expect(normalizeVersion("0.60.0")).toBe("0.60.0");
    });

    it("takes the leading dotted-numeric run out of a decorated string", () => {
        // `claude --version` prints "2.1.218 (Claude Code)".
        expect(normalizeVersion("2.1.218 (Claude Code)")).toBe("2.1.218");
        expect(normalizeVersion("codex-cli 0.154.0")).toBeUndefined();
    });

    it("returns undefined for absent or unparseable input", () => {
        expect(normalizeVersion(undefined)).toBeUndefined();
        expect(normalizeVersion("")).toBeUndefined();
        expect(normalizeVersion("unknown")).toBeUndefined();
    });
});

describe("compareVersions", () => {
    it("orders numerically, not lexically — the bug string equality hid", () => {
        // "2.1.9" > "2.1.10" under string compare; that is the whole point.
        expect(compareVersions("2.1.9", "2.1.10")).toBeLessThan(0);
        expect(compareVersions("2.1.274", "2.1.263")).toBeGreaterThan(0);
    });

    it("treats missing trailing segments as zero", () => {
        expect(compareVersions("2.1", "2.1.0")).toBe(0);
    });

    it("is reflexive on equal versions", () => {
        expect(compareVersions("0.60.0", "0.60.0")).toBe(0);
    });
});

describe("resolveDrift — installed vs pinned", () => {
    it("behind-pin: the live claude case (2.1.218 installed, 2.1.274 pinned)", () => {
        const r = resolveDrift({ installed: "2.1.218 (Claude Code)", pinned: "2.1.274", found: true });
        expect(r.drift).toBe("behind-pin");
        expect(isActionable(r)).toBe(true);
    });

    it("current when they match, even with a v prefix on one side", () => {
        expect(resolveDrift({ installed: "v0.60.0", pinned: "0.60.0", found: true }).drift).toBe("current");
    });

    it("ahead-of-pin is reported, but is not actionable by the user", () => {
        const r = resolveDrift({ installed: "2.1.300", pinned: "2.1.274", found: true });
        expect(r.drift).toBe("ahead-of-pin");
        expect(isActionable(r)).toBe(false);
    });

    it("not-installed wins over any version reasoning", () => {
        const r = resolveDrift({ pinned: "0.154.0", found: false });
        expect(r.drift).toBe("not-installed");
        expect(isActionable(r)).toBe(true);
    });

    it("unknown — not 'current' — when there is no pin (every system tool)", () => {
        // Must not render a reassuring 'up to date' for something we never pinned.
        expect(resolveDrift({ installed: "v22.22.2", found: true }).drift).toBe("unknown");
    });

    it("unknown when the tool reports an unparseable version", () => {
        expect(resolveDrift({ installed: "unknown", pinned: "1.0.0", found: true }).drift).toBe("unknown");
    });
});

describe("resolveDrift — pin vs upstream (AgentMux's problem, not the user's)", () => {
    it("pin-behind-upstream is independent of the installed version", () => {
        const r = resolveDrift({ installed: "2.1.274", pinned: "2.1.274", latest: "2.1.300", found: true });
        expect(r.drift).toBe("current"); // user is fine
        expect(r.currency).toBe("pin-behind-upstream"); // we are not
        expect(isActionable(r)).toBe(false); // and they cannot fix it
    });

    it("pin-current when the pin matches upstream", () => {
        const r = resolveDrift({ installed: "0.154.0", pinned: "0.154.0", latest: "0.154.0", found: true });
        expect(r.currency).toBe("pin-current");
    });

    it("a pin AHEAD of upstream still reads pin-current, never behind", () => {
        const r = resolveDrift({ installed: "1.0.0", pinned: "2.0.0", latest: "1.5.0", found: true });
        expect(r.currency).toBe("pin-current");
    });

    it("unknown currency when the registry lookup did not land — offline is normal", () => {
        const r = resolveDrift({ installed: "2.1.274", pinned: "2.1.274", found: true });
        expect(r.currency).toBe("unknown");
    });

    it("both axes resolve independently: behind the pin AND the pin behind upstream", () => {
        const r = resolveDrift({ installed: "2.1.218", pinned: "2.1.263", latest: "2.1.274", found: true });
        expect(r.drift).toBe("behind-pin");
        expect(r.currency).toBe("pin-behind-upstream");
    });
});
