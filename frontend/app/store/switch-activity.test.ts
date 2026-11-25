// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { dirtyAtClick, noteSwitchActivity, resetSwitchActivity } from "./switch-activity";

describe("switch-activity", () => {
    beforeEach(() => resetSwitchActivity());

    it("counts what arrived in the second before the click, most frequent first", () => {
        noteSwitchActivity("waveobj:update", 300);
        noteSwitchActivity("waveobj:update", 400);
        noteSwitchActivity("blockfile", 500);
        noteSwitchActivity("vault_op", 50);
        noteSwitchActivity("sysinfo", 1700);
        const dirty = dirtyAtClick(1200);
        expect(dirty.total).toBe(3);
        expect(dirty.mix).toBe("waveobj:update:2,blockfile:1");
    });

    it("says nothing when nothing arrived", () => {
        expect(dirtyAtClick(5000)).toEqual({ total: 0, mix: "" });
    });
});
