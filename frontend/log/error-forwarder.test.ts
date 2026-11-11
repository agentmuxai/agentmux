// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn() }));
import { extractErrorFields, isBenignBrowserNotice } from "./error-forwarder";

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

describe("extractErrorFields", () => {
    it("describes an event used as a rejection reason, which stringifies to nothing useful", () => {
        const ev = new Event("error");
        Object.defineProperty(ev, "target", { value: { tagName: "IMG", src: "https://example.com/favicon.ico" } });
        expect(extractErrorFields(ev)).toEqual({
            name: "Event",
            message: "error event on img https://example.com/favicon.ico",
            stack: null,
        });
    });

    it("keeps an Error's own name, message and stack, and stringifies anything else", () => {
        const err = new TypeError("x is not a function");
        expect(extractErrorFields(err)).toEqual({ name: "TypeError", message: "x is not a function", stack: err.stack });
        expect(extractErrorFields({ code: 7 })).toEqual({ name: null, message: '{"code":7}', stack: null });
    });
});
