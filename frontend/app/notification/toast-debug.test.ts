// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The text a toast's Copy button copies.

import { describe, expect, it } from "vitest";
import { errorDetail, toastDebugText } from "./toast-debug";

const toast = (extra: Partial<NotificationType> = {}): NotificationType => ({
    id: "t",
    icon: "fa-triangle-exclamation",
    title: "Couldn't show Agent1",
    message: "its tab did not open in time, and this sentence is long enough that a truncated toast would have cut it off",
    timestamp: "2026-11-02T05:12:03.045Z",
    type: "error",
    ...extra,
});

const env = { version: "0.60.12", build: "0.60.12", channel: "stable", platform: "windows x64", windowId: "win-1", tabId: "tab-9" };

describe("toastDebugText", () => {
    it("has the whole title and message, the kind and both times", () => {
        const text = toastDebugText(toast(), env);
        expect(text).toContain("Title: Couldn't show Agent1");
        expect(text).toContain("Message: its tab did not open in time, and this sentence is long enough that a truncated toast would have cut it off");
        expect(text).toContain("Kind: error");
        expect(text).toContain("Time (UTC): 2026-11-02T05:12:03.045Z");
        expect(text).toMatch(/Time: 2026-11-0[123] \d\d:\d\d:\d\d\.045 \(UTC[+-]\d\d:\d\d\)/);
    });

    it("names the build, channel, window and tab", () => {
        const text = toastDebugText(toast(), env);
        expect(text).toContain("AgentMux: 0.60.12, channel stable, windows x64");
        expect(text).toContain("Window: win-1");
        expect(text).toContain("Tab: tab-9");
    });

    it("names a build that differs from the version", () => {
        expect(toastDebugText(toast(), { ...env, build: "0.60.12-dev.4" })).toContain("AgentMux: 0.60.12, build 0.60.12-dev.4, channel stable");
    });

    it("adds the pane, the agent, the code, the request id and the detail the caller passed", () => {
        const text = toastDebugText(
            toast({ debug: { blockId: "blk-7", agentId: "agent1", agentName: "Agent1", code: "reveal:tab-timeout", requestId: "r-1", detail: { source: "swarm", n: 2 } } }),
            env
        );
        expect(text).toContain("Pane: blk-7");
        expect(text).toContain("Agent: Agent1 (agent1)");
        expect(text).toContain("Code: reveal:tab-timeout");
        expect(text).toContain("Request id: r-1");
        expect(text).toContain('"source": "swarm"');
    });

    it("lets the caller's window and tab win over the ambient ones", () => {
        const text = toastDebugText(toast({ debug: { windowId: "win-2", tabId: "tab-3" } }), env);
        expect(text).toContain("Window: win-2");
        expect(text).toContain("Tab: tab-3");
        expect(text).not.toContain("win-1");
    });

    it("blanks values whose key looks like a secret, at any depth", () => {
        const text = toastDebugText(
            toast({ debug: { detail: { token: "abc123", nested: { Authorization: "Bearer xyz", apiKey: "k", ok: "visible" }, list: [{ password: "pw" }] } } }),
            env
        );
        for (const secret of ["abc123", "Bearer xyz", "pw"]) expect(text).not.toContain(secret);
        expect(text).toContain("visible");
        expect(text).toContain("[hidden]");
    });

    it("does not copy what is nested deeper than it looks, secret or not", () => {
        let deep: Record<string, unknown> = { password: "deep-secret" };
        for (let i = 0; i < 12; i++) deep = { next: deep };
        const text = toastDebugText(toast({ debug: { detail: deep } }), env);
        expect(text).not.toContain("deep-secret");
        expect(text).toContain("[too deep]");
    });

    it("cuts a very long detail", () => {
        const text = toastDebugText(toast({ debug: { detail: { blob: "x".repeat(20000) } } }), env);
        expect(text.length).toBeLessThan(6000);
        expect(text).toContain("cut at 4000 characters");
    });

    it("copes with a toast that has only a title, no env and no time", () => {
        const text = toastDebugText({ icon: "bell", title: "Hi", message: "", timestamp: "" }, {});
        expect(text).toBe("Title: Hi\nKind: info");
    });

    it("prints an Error's name, message and stack through errorDetail", () => {
        const text = toastDebugText(toast({ debug: { detail: errorDetail(new TypeError("boom")) } }), env);
        expect(text).toContain('"error": "TypeError"');
        expect(text).toContain('"message": "boom"');
        expect(errorDetail("plain")).toEqual({ error: "plain" });
    });
});
