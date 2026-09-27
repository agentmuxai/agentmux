// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { limitLevel, trayLayout } from "./tray-layout";

describe("trayLayout", () => {
    it("shows every tile when they fit", () => {
        // 5 tiles × 64 + 4 gaps × 6 = 344
        expect(trayLayout(5, 344)).toEqual({ visible: 5, overflow: 0 });
        expect(trayLayout(1, 10)).toEqual({ visible: 1, overflow: 0 });
    });

    it("turns the last slot into +N when they don't", () => {
        // 344 px fits 5 slots: 4 tiles and a "+3".
        expect(trayLayout(7, 344)).toEqual({ visible: 4, overflow: 3 });
        expect(trayLayout(128, 344)).toEqual({ visible: 4, overflow: 124 });
    });

    it("always leaves room for the +N tile on a very narrow pane", () => {
        expect(trayLayout(3, 30)).toEqual({ visible: 0, overflow: 3 });
    });

    it("is empty with nothing attached", () => {
        expect(trayLayout(0, 500)).toEqual({ visible: 0, overflow: 0 });
    });
});

describe("limitLevel", () => {
    it("warns from 80% and errors past the limit", () => {
        expect(limitLevel(100, 128)).toBe("ok");
        expect(limitLevel(103, 128)).toBe("warn");
        expect(limitLevel(128, 128)).toBe("warn");
        expect(limitLevel(129, 128)).toBe("over");
    });
});
