// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { forgetWindowId, reattachableWindow, rememberWindowId, replacedWindow } from "./reload-window-id";

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

describe("replacedWindow: a window recreated after a crash opens onto the workspace the old one had", () => {
    const getWindow = vi.fn();
    const getWorkspace = vi.fn();
    const recreated = "?ipc_port=1&windowLabel=window-new&restoring=1&reprojectOf=w1";
    beforeEach(() => {
        getWindow.mockReset().mockResolvedValue(row("w1", "window-old", "ws7"));
        getWorkspace.mockReset().mockResolvedValue({ oid: "ws7" });
    });

    it("returns the old row, whose label is another window's, when it is still open and has its workspace", async () => {
        await expect(replacedWindow(["main", "w1"], getWindow, getWorkspace, recreated)).resolves.toMatchObject({
            oid: "w1",
            workspaceid: "ws7",
        });
        expect(getWindow).toHaveBeenCalledWith("w1");
        expect(getWorkspace).toHaveBeenCalledWith("ws7");
    });

    it("any other window replaces nothing, without asking", async () => {
        await expect(replacedWindow(["main", "w1"], getWindow, getWorkspace, "?windowLabel=window-new")).resolves.toBeUndefined();
        await expect(
            replacedWindow(["main", "w1"], getWindow, getWorkspace, "?windowLabel=window-new&restoring=1")
        ).resolves.toBeUndefined();
        expect(getWindow).not.toHaveBeenCalled();
    });

    it("an old row that is closed already isn't looked up", async () => {
        await expect(replacedWindow(["main"], getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
        await expect(replacedWindow(undefined, getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
        expect(getWindow).not.toHaveBeenCalled();
    });

    it("a row with no workspace, a workspace that is gone, or a failed lookup falls back to a new workspace", async () => {
        getWindow.mockResolvedValueOnce(row("w1", "window-old", ""));
        await expect(replacedWindow(["w1"], getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
        getWorkspace.mockResolvedValueOnce(null);
        await expect(replacedWindow(["w1"], getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
        getWindow.mockRejectedValueOnce(new Error("timeout"));
        await expect(replacedWindow(["w1"], getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
        getWindow.mockResolvedValueOnce(null);
        await expect(replacedWindow(["w1"], getWindow, getWorkspace, recreated)).resolves.toBeUndefined();
    });
});
