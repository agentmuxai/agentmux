// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn() }));
import { isBenignBrowserNotice } from "./error-forwarder";

describe("isBenignBrowserNotice", () => {
    it("knows the ResizeObserver notices, in both engine wordings", () => {
        expect(isBenignBrowserNotice("ResizeObserver loop completed with undelivered notifications.")).toBe(true);
        expect(isBenignBrowserNotice("ResizeObserver loop limit exceeded")).toBe(true);
    });

    it("lets every other error through", () => {
        expect(isBenignBrowserNotice("Uncaught TypeError: x is not a function")).toBe(false);
        expect(isBenignBrowserNotice("The ResizeObserver loop completed with undelivered notifications.")).toBe(false);
        expect(isBenignBrowserNotice("")).toBe(false);
        expect(isBenignBrowserNotice(undefined)).toBe(false);
    });
});
