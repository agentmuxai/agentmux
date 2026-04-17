// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, afterEach } from "vitest";
import { isHostApp } from "./host-detect";

describe("isHostApp", () => {
    afterEach(() => {
        delete (window as any).__AGENTMUX_IPC_PORT__;
    });

    it("returns true when __AGENTMUX_IPC_PORT__ is set", () => {
        (window as any).__AGENTMUX_IPC_PORT__ = "12345";
        expect(isHostApp()).toBe(true);
    });

    it("returns true when __AGENTMUX_IPC_PORT__ is 0", () => {
        (window as any).__AGENTMUX_IPC_PORT__ = 0;
        expect(isHostApp()).toBe(true);
    });

    it("returns false when __AGENTMUX_IPC_PORT__ is undefined", () => {
        expect(isHostApp()).toBe(false);
    });
});
