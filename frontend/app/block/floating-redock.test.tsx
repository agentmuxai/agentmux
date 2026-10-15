// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    instances: [] as { label: string; windowId: string | null }[],
    listFails: false,
    workspace: { oid: "fws" } as { oid: string } | undefined,
    activeTab: "ftab" as string | undefined,
    objects: new Map<string, unknown>(),
    reloads: [] as string[],
    redocks: [] as unknown[][],
    redockFails: false,
    focused: [] as string[],
}));

vi.mock("@/store/global", () => ({
    atoms: { workspace: () => hub.workspace, activeTabId: () => hub.activeTab },
    getApi: () => ({
        listWindowInstances: async () => {
            if (hub.listFails) throw new Error("no host");
            return hub.instances;
        },
        focusWindow: async (label: string) => void hub.focused.push(label),
    }),
}));
vi.mock("@/app/store/mos", () => ({
    makeORef: (t: string, id: string) => `${t}:${id}`,
    reloadMuxObject: async (oref: string) => {
        hub.reloads.push(oref);
        const o = hub.objects.get(oref);
        if (!o) throw new Error("missing " + oref);
        return o;
    },
    getObjectValue: (oref: string) => hub.objects.get(oref),
}));
vi.mock("@/app/store/services", () => ({
    WorkspaceService: {
        RedockFloatingPane: async (...args: unknown[]) => {
            hub.redocks.push(args);
            if (hub.redockFails) throw new Error("saga");
            return { redocked: true };
        },
    },
}));

import { FloatingRedockButton, findRedockTarget, pickRedockTarget, redockFloaterInto, redockThisFloater } from "./floating-redock";

const setLabel = (label: string) => window.history.replaceState({}, "", `/?windowLabel=${label}`);

beforeEach(() => {
    hub.instances = [{ label: "main", windowId: "w-main" }, { label: "floating-1", windowId: "w-f" }];
    hub.listFails = false;
    hub.workspace = { oid: "fws" };
    hub.activeTab = "ftab";
    hub.objects = new Map<string, unknown>([
        ["window:w-main", { workspaceid: "mws" }],
        ["workspace:mws", { oid: "mws", activetabid: "mtab" }],
        ["tab:ftab", { blockids: ["b1"] }],
    ]);
    hub.reloads = [];
    hub.redocks = [];
    hub.redockFails = false;
    hub.focused = [];
    setLabel("floating-1");
});
afterEach(() => {
    cleanup();
    setLabel("main");
});

describe("pickRedockTarget", () => {
    it("prefers the main window", () => {
        expect(pickRedockTarget([{ label: "window-a", windowId: "wa" }, { label: "main", windowId: "wm" }])).toEqual({ label: "main", windowId: "wm" });
    });
    it("falls back to another full window when main is gone", () => {
        expect(pickRedockTarget([{ label: "floating-1", windowId: "wf" }, { label: "window-a", windowId: "wa" }])).toEqual({ label: "window-a", windowId: "wa" });
    });
    it("never picks a floater, a browser-pane window, or a window that has no backend id yet", () => {
        expect(
            pickRedockTarget([
                { label: "floating-1", windowId: "wf" },
                { label: "browser-pane-x", windowId: "wb" },
                { label: "window-pool-1", windowId: null },
            ])
        ).toBeNull();
        expect(pickRedockTarget([])).toBeNull();
    });
    it("findRedockTarget is null when the host cannot list windows", async () => {
        hub.listFails = true;
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(await findRedockTarget()).toBeNull();
        err.mockRestore();
    });
});

describe("redockFloaterInto", () => {
    it("redocks the floater's block into the target's active tab, reading the target on demand", async () => {
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(true);
        expect(hub.reloads).toEqual(["window:w-main", "workspace:mws"]);
        expect(hub.redocks).toEqual([["b1", "ftab", "fws", "mtab", "mws"]]);
    });
    it("redocks every block of a floater that holds more than one", async () => {
        hub.objects.set("tab:ftab", { blockids: ["b1", "b2"] });
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(true);
        expect(hub.redocks.map((a) => a[0])).toEqual(["b1", "b2"]);
    });
    it("does nothing when the target has no active tab, or cannot be read", async () => {
        hub.objects.set("workspace:mws", { oid: "mws", activetabid: "" });
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(false);
        warn.mockRestore();
        hub.objects.delete("window:w-main");
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(false);
        err.mockRestore();
        expect(hub.redocks).toEqual([]);
    });
    it("reports a failed saga as not redocked", async () => {
        hub.redockFails = true;
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(false);
        err.mockRestore();
    });
    it("needs the floater's own workspace and tab", async () => {
        hub.activeTab = undefined;
        expect(await redockFloaterInto({ label: "main", windowId: "w-main" })).toBe(false);
        expect(hub.redocks).toEqual([]);
    });
});

describe("redockThisFloater", () => {
    it("redocks, then brings the target window forward", async () => {
        expect(await redockThisFloater()).toBe(true);
        expect(hub.redocks).toHaveLength(1);
        expect(hub.focused).toEqual(["main"]);
    });
    it("does nothing in a window that is not a floater", async () => {
        setLabel("main");
        expect(await redockThisFloater()).toBe(false);
        expect(hub.redocks).toEqual([]);
    });
    it("does nothing, and moves no window, when there is nowhere to dock", async () => {
        hub.instances = [{ label: "floating-1", windowId: "w-f" }];
        expect(await redockThisFloater()).toBe(false);
        expect(hub.focused).toEqual([]);
    });
    it("does not bring a window forward when the redock failed", async () => {
        hub.redockFails = true;
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(await redockThisFloater()).toBe(false);
        err.mockRestore();
        expect(hub.focused).toEqual([]);
    });
});

describe("FloatingRedockButton", () => {
    it("shows a Redock button with the tile glyph when a window can take the pane", async () => {
        const { container } = render(() => <FloatingRedockButton />);
        await waitFor(() => expect(container.querySelector("button")).not.toBeNull());
        const btn = container.querySelector("button")!;
        expect(btn.getAttribute("title")).toBe("Redock");
        expect(btn.querySelector("svg.pane-restore-glyph")).not.toBeNull();
    });
    it("is hidden when there is no window to dock to", async () => {
        hub.instances = [{ label: "floating-1", windowId: "w-f" }];
        const { container } = render(() => <FloatingRedockButton />);
        await new Promise((r) => setTimeout(r, 20));
        expect(container.querySelector("button")).toBeNull();
    });
    it("clicking it redocks and brings the main window forward", async () => {
        const { container } = render(() => <FloatingRedockButton />);
        await waitFor(() => expect(container.querySelector("button")).not.toBeNull());
        fireEvent.mouseDown(container.querySelector("button")!);
        fireEvent.mouseUp(container.querySelector("button")!);
        fireEvent.click(container.querySelector("button")!);
        await waitFor(() => expect(hub.redocks).toHaveLength(1));
        await waitFor(() => expect(hub.focused).toEqual(["main"]));
    });
    it("hides itself when the click finds the target gone", async () => {
        const { container } = render(() => <FloatingRedockButton />);
        await waitFor(() => expect(container.querySelector("button")).not.toBeNull());
        hub.instances = [{ label: "floating-1", windowId: "w-f" }];
        fireEvent.mouseDown(container.querySelector("button")!);
        fireEvent.mouseUp(container.querySelector("button")!);
        fireEvent.click(container.querySelector("button")!);
        await waitFor(() => expect(container.querySelector("button")).toBeNull());
        expect(hub.redocks).toEqual([]);
    });
});
