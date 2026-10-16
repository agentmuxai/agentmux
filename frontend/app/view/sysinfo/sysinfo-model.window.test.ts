// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The pane's own window and area, kept in its meta (the window ahead of the global setting), and whether it is on screen. */
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({
    read: vi.fn(async (..._args: unknown[]): Promise<any[]> => []),
    settings: {} as Record<string, unknown>,
}));

vi.mock("@/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ settings: env.settings }) },
    getConnStatusAtom: () => () => ({ status: "connected" }),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { EventReadHistoryCommand: env.read } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { SysinfoViewModel } from "./sysinfo-model";
import { DefaultNumPoints } from "./sysinfo-types";

function model(meta: Record<string, unknown> = {}, visibility: () => string = () => "active") {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => setM({ ...m(), ...patch }));
    const ctx = { blockId: "s1", meta: m, setMeta, isFocused: () => false, visibility };
    return { model: new SysinfoViewModel(ctx as unknown as PaneTabHostContext, "sysinfo"), setMeta };
}

beforeEach(() => {
    env.read.mockClear();
    env.settings = {};
});

afterEach(() => vi.restoreAllMocks());

describe("SysinfoViewModel window", () => {
    it("is the default with nothing chosen, and the global setting when there is one", () => {
        createRoot((dispose) => {
            expect(model().model.numPoints()).toBe(DefaultNumPoints);
            env.settings = { "telemetry:numpoints": 300 };
            expect(model().model.numPoints()).toBe(300);
            dispose();
        });
    });

    it("prefers the window this pane chose over the global setting", () => {
        createRoot((dispose) => {
            env.settings = { "telemetry:numpoints": 300 };
            expect(model({ "graph:numpoints": 60 }).model.numPoints()).toBe(60);
            dispose();
        });
    });

    it("takes a new window at once, saves it as a time, and reads enough history to fill it", async () => {
        await createRoot(async (dispose) => {
            const { model: m, setMeta } = model();
            m.setWindow(900);
            expect(m.numPoints()).toBe(900);
            expect(m.windowSecs()).toBe(900);
            expect(setMeta).toHaveBeenCalledWith({ "graph:windowsecs": 900 });
            await Promise.resolve();
            const last = env.read.mock.calls[env.read.mock.calls.length - 1];
            expect((last[1] as { maxitems: number }).maxitems).toBe(900);
            dispose();
        });
    });

    it("spans the chosen time at any sample interval", () => {
        createRoot((dispose) => {
            env.settings = { "telemetry:interval": 2 };
            const { model: m } = model({ "graph:windowsecs": 300 });
            expect(m.numPoints()).toBe(150);
            expect(m.windowSecs()).toBe(300);
            dispose();
        });
    });

    it("keeps the newest load's history when an older, slower one finishes after it", async () => {
        await createRoot(async (dispose) => {
            const pending: ((items: any[]) => void)[] = [];
            env.read.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
            const { model: m } = model();
            await Promise.resolve();
            m.setWindow(900);
            await Promise.resolve();
            const at = (ts: number) => ({ event: "sysinfo", scopes: ["local"], data: { ts, values: { cpu: 1 } } });
            // The 900-sample load answers first, then the pane's first, shorter one.
            pending[1]([at(1_000), at(2_000), at(3_000)]);
            await new Promise((r) => setTimeout(r, 0));
            pending[0]([at(3_000)]);
            await new Promise((r) => setTimeout(r, 0));
            expect(m.dataAtom().filter((d) => d.blank !== 1)).toHaveLength(3);
            expect(m.loadingAtom()).toBe(false);
            env.read.mockImplementation(async () => []);
            dispose();
        });
    });

    it("keeps a window inside what srv's history holds", () => {
        createRoot((dispose) => {
            const { model: m } = model();
            m.setWindow(5000);
            expect(m.numPoints()).toBe(1024);
            dispose();
        });
    });
});

describe("SysinfoViewModel focus", () => {
    it("shows CPU until an area is chosen, and reads one saved in the meta", () => {
        createRoot((dispose) => {
            expect(model().model.focus()).toBe("cpu");
            expect(model().model.viewName()).toBe("CPU");
            expect(model({ "sysinfo:focus": "mem" }).model.focus()).toBe("mem");
            expect(model({ "sysinfo:focus": "bogus" }).model.focus()).toBe("cpu");
            dispose();
        });
    });

    it("keeps a pane that saved an area but no plot type on the area view", () => {
        createRoot((dispose) => {
            const { model: m } = model({ "sysinfo:focus": "mem" });
            expect(m.plotTypeSelectedAtom()).toBe("Overview");
            expect(m.viewName()).toBe("Memory");
            expect(model({ "sysinfo:focus": "advanced" }).model.viewName()).toBe("Everything");
            // Nothing saved: simple CPU.
            expect(model().model.plotTypeSelectedAtom()).toBe("CPU");
            dispose();
        });
    });

    it("is visible only while its pane is active", () => {
        createRoot((dispose) => {
            const [vis, setVis] = createSignal("active");
            const { model: m } = model({}, vis);
            expect(m.visible()).toBe(true);
            setVis("dormant");
            expect(m.visible()).toBe(false);
            setVis("windowHidden");
            expect(m.visible()).toBe(false);
            dispose();
        });
    });

    it("takes an area at once, saves it, and is named for it", () => {
        createRoot((dispose) => {
            const { model: m, setMeta } = model({ "sysinfo:type": "Overview" });
            m.setFocus("net");
            expect(m.focus()).toBe("net");
            expect(m.viewName()).toBe("Network");
            expect(setMeta).toHaveBeenCalledWith({ "sysinfo:focus": "net" });
            m.setFocus("everything");
            expect(m.viewName()).toBe("Everything");
            expect(setMeta).toHaveBeenLastCalledWith({ "sysinfo:focus": "everything" });
            dispose();
        });
    });

    it("names a simple view by its label, whatever area is saved", () => {
        createRoot((dispose) => {
            expect(model({ "sysinfo:type": "Mem", "sysinfo:focus": "cpu" }).model.viewName()).toBe("Memory");
            dispose();
        });
    });
});

describe("SysinfoViewModel menus", () => {
    const sub = (items: ContextMenuItem[], label: string) => items.find((i) => i.label === label)?.submenu ?? [];

    it("offer a Simple submenu, then an Advanced one with the area buttons' choices", () => {
        createRoot((dispose) => {
            const { model: m } = model({ "sysinfo:type": "Overview", "sysinfo:focus": "mem" });
            const items = m.getBodyContextMenuItems();
            expect(items.map((i) => i.label)).toEqual(["Simple", "Advanced"]);
            const advanced = sub(items, "Advanced");
            expect(advanced.map((i) => i.label)).toEqual(["CPU", "Memory", "Disk", "Network", "Everything"]);
            expect(advanced.filter((i) => i.checked).map((i) => i.label)).toEqual(["Memory"]);
            expect(sub(items, "Simple").map((i) => i.label)).toEqual([
                "CPU",
                "All cores",
                "Memory",
                "Network",
                "Network sent and received",
                "Disk",
                "Disk read and write",
                "CPU and memory",
                "CPU, memory and network",
            ]);
            // The settings menu's Plot Type holds the same two.
            const plotType = sub(m.getSettingsMenuItems(), "Plot Type");
            expect(plotType.map((i) => i.label)).toEqual(["Simple", "Advanced"]);
            dispose();
        });
    });

    it("check the pane's simple view, and show a picked one", async () => {
        await createRoot(async (dispose) => {
            const { model: m, setMeta } = model({ "sysinfo:type": "All CPU" });
            const items = m.getBodyContextMenuItems();
            expect(sub(items, "Simple").filter((i) => i.checked).map((i) => i.label)).toEqual(["All cores"]);
            expect(sub(items, "Advanced").some((i) => i.checked)).toBe(false);
            await sub(items, "Simple").find((i) => i.label === "Memory")!.click!();
            expect(setMeta).toHaveBeenLastCalledWith(expect.objectContaining({ "sysinfo:type": "Mem" }));
            expect(m.viewName()).toBe("Memory");
            dispose();
        });
    });

    it("show the picked area, leaving a simple view", async () => {
        await createRoot(async (dispose) => {
            const { model: m, setMeta } = model({ "sysinfo:type": "Mem" });
            await sub(m.getBodyContextMenuItems(), "Advanced").find((i) => i.label === "Everything")!.click!();
            expect(setMeta).toHaveBeenLastCalledWith({ "sysinfo:type": "Overview", "sysinfo:focus": "everything" });
            expect(m.viewName()).toBe("Everything");
            dispose();
        });
    });
});

describe("SysinfoViewModel controls", () => {
    it("hides the view and window buttons until the pane's meta says to show them", () => {
        createRoot((dispose) => {
            expect(model().model.controlsShown()).toBe(false);
            expect(model({ "sysinfo:showcontrols": false }).model.controlsShown()).toBe(false);
            expect(model({ "sysinfo:showcontrols": true }).model.controlsShown()).toBe(true);
            dispose();
        });
    });

    it("takes a choice at once and saves it in the pane's meta", () => {
        createRoot((dispose) => {
            const { model: m, setMeta } = model();
            m.setControlsShown(true);
            expect(m.controlsShown()).toBe(true);
            expect(setMeta).toHaveBeenCalledWith({ "sysinfo:showcontrols": true });
            m.setControlsShown(false);
            expect(m.controlsShown()).toBe(false);
            dispose();
        });
    });
});
