// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: vi.fn(async () => {}) }));
vi.mock("@/app/store/global", () => ({ atoms: { fullConfigAtom: () => ({ widgets: {} }) } }));

import type { PaneTabInfo } from "./pane-tab-model";
import { persistTabInfo, PERSIST_DELAY_MS, resetTabPersistForTests } from "./pane-tab-persist";

const info = (blockId: string, persist?: PaneTabInfo["persist"]): PaneTabInfo =>
    ({ blockId, label: "x", icon: { kind: "fa", name: "globe" }, persist }) as PaneTabInfo;

describe("persistTabInfo", () => {
    const write = vi.fn(async () => {});
    beforeEach(() => {
        vi.useFakeTimers();
        write.mockClear();
    });
    afterEach(() => {
        resetTabPersistForTests();
        vi.useRealTimers();
    });

    it("writes the name and favicon to the block's meta after the delay", async () => {
        persistTabInfo(info("b1", { name: "Docs", favicon: "https://d/f.ico" }), write);
        expect(write).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(PERSIST_DELAY_MS);
        expect(write).toHaveBeenCalledWith("b1", { "pane:label": "Docs", "pane:favicon": "https://d/f.ico" });
    });

    it("sends only the newest value when it changes while a write is waiting, and does not push the write back", async () => {
        persistTabInfo(info("b1", { name: "One" }), write);
        await vi.advanceTimersByTimeAsync(PERSIST_DELAY_MS - 100);
        persistTabInfo(info("b1", { name: "Two" }), write);
        await vi.advanceTimersByTimeAsync(100);
        expect(write).toHaveBeenCalledTimes(1);
        expect(write).toHaveBeenCalledWith("b1", { "pane:label": "Two" });
    });

    it("drops a waiting write once the meta already holds the name", async () => {
        persistTabInfo(info("b1", { name: "One" }), write);
        persistTabInfo(info("b1", undefined), write);
        await vi.advanceTimersByTimeAsync(PERSIST_DELAY_MS * 2);
        expect(write).not.toHaveBeenCalled();
    });

    it("writes nothing for a tab with nothing to keep, and each tab has its own write", async () => {
        persistTabInfo(info("b1"), write);
        persistTabInfo(info("b2", { name: "Two" }), write);
        persistTabInfo(info("b3", { name: "Three" }), write);
        await vi.advanceTimersByTimeAsync(PERSIST_DELAY_MS);
        expect(write).toHaveBeenCalledTimes(2);
        expect(write).toHaveBeenCalledWith("b2", { "pane:label": "Two" });
        expect(write).toHaveBeenCalledWith("b3", { "pane:label": "Three" });
    });
});
