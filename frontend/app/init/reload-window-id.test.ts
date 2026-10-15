// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { forgetWindowId, reattachableWindow, rememberWindowId } from "./reload-window-id";

const row = (oid: string, label: string, workspaceid = "ws1") =>
    ({ otype: "window", oid, version: 1, meta: { "host:label": label }, workspaceid }) as unknown as MuxWindow;

describe("reattachableWindow: a reload re-attaches to the window's own row", () => {
    const getWindow = vi.fn();
    const getWorkspace = vi.fn();
    beforeEach(() => {
        sessionStorage.clear();
        getWindow.mockReset().mockResolvedValue(row("w1", "window-abc"));
        getWorkspace.mockReset().mockResolvedValue({ oid: "ws1" });
    });

    it("returns the previous row when it is still open, still this window's, and has its workspace", async () => {
        rememberWindowId("w1");
        await expect(reattachableWindow(["main", "w1"], "window-abc", getWindow, getWorkspace)).resolves.toMatchObject({ oid: "w1" });
        expect(getWorkspace).toHaveBeenCalledWith("ws1");
    });

    it("a first load (no previous row) creates a row, without asking", async () => {
        await expect(reattachableWindow(["main"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
        forgetWindowId(); // a pool renderer becoming another window
        await expect(reattachableWindow(["main", "w1"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
        expect(getWindow).not.toHaveBeenCalled();
    });

    it("a row that was closed (no longer in the client's windows) isn't reused", async () => {
        rememberWindowId("w1");
        await expect(reattachableWindow(["main"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
        expect(getWindow).not.toHaveBeenCalled();
    });

    it("a row belonging to another window label isn't reused", async () => {
        rememberWindowId("w1");
        await expect(reattachableWindow(["main", "w1"], "window-xyz", getWindow, getWorkspace)).resolves.toBeUndefined();
    });

    it("a row whose workspace is gone, or a failed lookup, falls back to a new row", async () => {
        rememberWindowId("w1");
        getWorkspace.mockResolvedValueOnce(null);
        await expect(reattachableWindow(["main", "w1"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
        getWindow.mockRejectedValueOnce(new Error("timeout"));
        await expect(reattachableWindow(["main", "w1"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
        getWindow.mockResolvedValueOnce(null);
        await expect(reattachableWindow(["main", "w1"], "window-abc", getWindow, getWorkspace)).resolves.toBeUndefined();
    });
});
