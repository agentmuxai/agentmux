// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const { rename, flush, updateMeta, push } = vi.hoisted(() => ({
    rename: vi.fn(async (_id: string, _name: string) => {}),
    flush: vi.fn(async () => ({ saved: true })),
    updateMeta: vi.fn(async (_oref: string, _meta: unknown) => {}),
    push: vi.fn(),
}));
vi.mock("./services", () => ({
    WorkspaceService: { RenameUserWorkspace: rename, Flush: flush },
    ObjectService: { UpdateObjectMeta: updateMeta },
}));
vi.mock("./mos", () => ({ makeORef: (otype: string, oid: string) => `${otype}:${oid}` }));
vi.mock("./flash-notifications", () => ({ pushNotification: push }));

import {
    DEFAULT_WORKSPACE_NAME,
    installWorkspaceFlushOnBlur,
    renameWorkspace,
    surfaceWorkspaceRestoreNotice,
    workspaceHeading,
    workspaceName,
} from "./user-workspace";

const client = (meta: Record<string, unknown>): Client =>
    ({ otype: "client", oid: "c1", version: 1, windowids: [], meta }) as unknown as Client;

afterEach(() => vi.clearAllMocks());

describe("the open workspace's name", () => {
    it("is the client's, or Default Workspace before srv says", () => {
        expect(workspaceName(client({ "workspace:name": "Reviews" }))).toBe("Reviews");
        expect(workspaceName(client({}))).toBe(DEFAULT_WORKSPACE_NAME);
        expect(workspaceName(client({ "workspace:name": "  " }))).toBe(DEFAULT_WORKSPACE_NAME);
        expect(workspaceName(null)).toBe(DEFAULT_WORKSPACE_NAME);
    });

    it("heads the window list with its window count", () => {
        expect(workspaceHeading("Default Workspace", 1)).toBe("Default Workspace — 1 window");
        expect(workspaceHeading("Reviews", 3)).toBe("Reviews — 3 windows");
        expect(workspaceHeading("Reviews", 0)).toBe("Reviews — 0 windows");
    });
});

describe("renameWorkspace", () => {
    it("renames the open workspace, trimmed", async () => {
        await renameWorkspace("  Reviews ");
        expect(rename).toHaveBeenCalledWith("", "Reviews");
    });

    it("sends nothing for a blank name", async () => {
        await renameWorkspace("   ");
        expect(rename).not.toHaveBeenCalled();
    });
});

describe("saving on focus loss", () => {
    it("flushes the workspace when the window loses focus, once per blur", () => {
        installWorkspaceFlushOnBlur();
        installWorkspaceFlushOnBlur(); // idempotent
        window.dispatchEvent(new Event("blur"));
        expect(flush).toHaveBeenCalledTimes(1);
    });
});

describe("the restore notice", () => {
    it("says once that an older copy opened, and clears the mark", () => {
        surfaceWorkspaceRestoreNotice(client({ "workspace:name": "Work", "workspace:restored_from": "previous" }));
        expect(push).toHaveBeenCalledTimes(1);
        const notice = push.mock.calls[0][0];
        expect(notice.title).toBe("Work opened from an older copy");
        expect(notice.message).toContain("the last copy that opened cleanly");
        expect(notice.type).toBe("warning");
        expect(updateMeta).toHaveBeenCalledWith("client:c1", { "workspace:restored_from": null });
    });

    it("names the pre-update copy", () => {
        surfaceWorkspaceRestoreNotice(client({ "workspace:restored_from": "pre_upgrade" }));
        expect(push.mock.calls[0][0].message).toContain("from before the update");
    });

    it("says nothing after a normal launch", () => {
        surfaceWorkspaceRestoreNotice(client({ "workspace:name": "Work" }));
        surfaceWorkspaceRestoreNotice(null);
        expect(push).not.toHaveBeenCalled();
        expect(updateMeta).not.toHaveBeenCalled();
    });
});
