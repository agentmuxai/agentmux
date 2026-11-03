// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    describeLeave,
    leavePrompts,
    notePersonClose,
    siteOf,
    takePersonClose,
    type LeaveProbe,
} from "./leave-prompt";

const probe = (over: Partial<LeaveProbe> = {}): LeaveProbe => ({
    isBrowser: (id) => id.startsWith("b"),
    asks: async (id) => id.endsWith("!"),
    site: (id) => `${id.replace("!", "")}.example.com`,
    ...over,
});

describe("leavePrompts", () => {
    it("lists the browser panes whose page would prompt, and only those", async () => {
        const got = await leavePrompts(["b1!", "b2", "t1!", "b3!"], probe());
        expect(got.map((p) => p.blockId)).toEqual(["b1!", "b3!"]);
        expect(got[0].site).toBe("b1.example.com");
    });

    it("asks nothing for a pane that is not a browser", async () => {
        let asked = 0;
        await leavePrompts(["t1!"], probe({ asks: async () => (asked++, true) }));
        expect(asked).toBe(0);
    });

    it("does not wait past its budget for a page that is slow to answer", async () => {
        vi.useFakeTimers();
        try {
            const never = new Promise<boolean>(() => {});
            const pending = leavePrompts(["b1", "b2!"], probe({ asks: (id) => (id === "b1" ? never : Promise.resolve(true)) }), 1000);
            await vi.advanceTimersByTimeAsync(999);
            let settled = false;
            void pending.then(() => (settled = true));
            await vi.advanceTimersByTimeAsync(0);
            expect(settled).toBe(false);
            await vi.advanceTimersByTimeAsync(1);
            // The slow page counts as "would not"; the one that answered keeps its prompt.
            expect((await pending).map((p) => p.blockId)).toEqual(["b2!"]);
        } finally {
            vi.useRealTimers();
        }
    });

    it("answers at once when every page answers within the budget", async () => {
        const got = await leavePrompts(["b1!"], probe(), 1000);
        expect(got.map((p) => p.blockId)).toEqual(["b1!"]);
    });

    it("never blocks a close on a check that failed", async () => {
        const got = await leavePrompts(["b1"], probe({ asks: async () => Promise.reject(new Error("no page")) }));
        expect(got).toEqual([]);
    });
});

describe("describeLeave and siteOf", () => {
    it("names the site", () => {
        expect(describeLeave([{ blockId: "b", site: "portal.azure.com" }])).toBe(
            "portal.azure.com may have unsaved changes."
        );
    });

    it("names several sites once each", () => {
        const p = (site: string) => ({ blockId: site, site });
        expect(describeLeave([p("a.com"), p("b.com"), p("a.com"), p("c.com")])).toBe(
            "a.com, b.com and c.com may have unsaved changes."
        );
    });

    it("reads a host name, never the path or query", () => {
        expect(siteOf("https://portal.azure.com/#view/x?token=1")).toBe("portal.azure.com");
        expect(siteOf("nonsense")).toBe("this page");
        expect(siteOf(undefined)).toBe("this page");
    });
});

describe("person closes", () => {
    it("are reported once, as the pane closes", () => {
        notePersonClose(["p1", "p2"]);
        expect(takePersonClose("p1")).toBe(true);
        expect(takePersonClose("p1")).toBe(false);
        expect(takePersonClose("p2")).toBe(true);
        expect(takePersonClose("never-noted")).toBe(false);
    });
});
