// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the InstancePanel does around its window rows, pinned as ordered logs and HTML: what it asks
 * the host for when it opens and in what order, the About rows and their copy buttons, how a window's
 * name and id are resolved (including this window's own row before its id arrives), a rename that is
 * still open when the panel goes away, the workspace heading's keys, a closed window's row, the
 * floating panes with their opacity control, and the footer. The window rows themselves are in
 * InstancePanel.rows.test.tsx, the heading, closed windows and slot order in InstancePanel.test.tsx.
 * A refactor of the panel must leave the committed snapshots as they are; regenerate with `vitest -u`
 * only for a change meant to behave differently.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    about: null as null | Record<string, unknown>,
    dev: false,
    launcherActive: true,
    label: "main" as string | Error,
    snapshot: Promise.resolve([] as unknown[]) as Promise<unknown[]>,
    caps: new Set<string>(),
    floating: [] as Array<Record<string, unknown>>,
    windows: {} as Record<string, { meta: Record<string, unknown>; workspaceid?: string }>,
    workspaces: {} as Record<string, { name: string; activetabid?: string }>,
    entries: [] as Array<{ label: string; windowId: string | null }>,
    ownOid: null as string | null,
    live: new Map<string, number>(),
    focusFails: false,
    openNewFails: false,
    updateMetaFails: false,
    renameFails: false,
    workspaceName: "Default Workspace",
    dismiss: (() => {}) as () => void,
}));
const log = (line: string) => h.log.push(line);

const services = vi.hoisted(() => () => ({
    ObjectService: {
        UpdateObjectMeta: async (oref: string, meta: unknown) => {
            h.log.push(`UpdateObjectMeta ${oref} ${JSON.stringify(meta)}`);
            if (h.updateMetaFails) throw new Error("meta refused");
        },
    },
    WorkspaceService: {
        RenameUserWorkspace: async (_id: string, name: string) => {
            h.log.push(`RenameUserWorkspace ${JSON.stringify(name)}`);
            if (h.renameFails) throw new Error("rename refused");
        },
        Flush: () => {},
        ListUserWorkspaces: async () => [],
    },
    WindowService: {
        ListClosedWindows: async () => {
            h.log.push("ListClosedWindows");
            return [];
        },
        PlaceWindowSlots: async (keys: string[]) => {
            h.log.push(`PlaceWindowSlots ${JSON.stringify(keys)}`);
            return keys;
        },
    },
}));


vi.mock("@/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [entries, setEntries] = createSignal<Array<{ label: string; windowId: string | null }>>([]);
    (globalThis as any).__setPanelEntries = setEntries;
    return {
        atoms: {
            client: () => ({ otype: "client", oid: "c1", version: 1, windowids: [], meta: { "workspace:current": "w1", "workspace:name": h.workspaceName } }),
            muxWindow: () => (h.ownOid ? { oid: h.ownOid } : null),
        },
        getApi: () => ({
            getAboutModalDetails: () => {
                log("api getAboutModalDetails");
                return h.about;
            },
            getWindowLabel: async () => {
                log("api getWindowLabel");
                if (h.label instanceof Error) throw h.label;
                return h.label;
            },
            getDoubleClickTime: async () => 500,
            listWindowInstances: () => {
                log("api listWindowInstances");
                return h.snapshot;
            },
            focusWindow: async (label: string) => {
                log(`api focusWindow ${label}`);
                if (h.focusFails) throw new Error("no such window");
            },
        }),
        isDev: () => h.dev,
        openWindowEntriesAtom: () => entries(),
        openFloatingPaneEntriesAtom: () => h.floating,
    };
});
vi.mock("./MaintenanceSection", () => ({
    MaintenanceSection: () => {
        const el = document.createElement("div");
        el.className = "maintenance-stub";
        return el;
    },
}));
vi.mock("@/util/host-caps", () => ({ hostHas: (k: string) => h.caps.has(k) }));
vi.mock("@/app/store/launcher-event-store", () => ({
    reconcileKnownEntriesFromSnapshot: (s: unknown) => log(`reconcile ${JSON.stringify(s)}`),
}));
vi.mock("@/util/launcher-events", () => ({ launcherEventsActive: () => h.launcherActive }));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: { children: unknown; onDismiss: () => void; class?: string; role?: string; "aria-label"?: string; placement?: string; style?: Record<string, string> }) => {
        h.dismiss = p.onDismiss;
        log(`popover class=${p.class} role=${p.role} label=${p["aria-label"]} placement=${p.placement} width=${p.style?.width}`);
        return <div data-testid="panel">{p.children as any}</div>;
    },
}));
vi.mock("@/util/clipboard", () => ({ writeText: (t: string) => log(`copy ${JSON.stringify(t)}`) }));
const getObject = (oref: string) => {
    if (oref.startsWith("window:")) {
        const id = oref.slice("window:".length);
        return h.windows[id] ? { oid: id, ...h.windows[id] } : null;
    }
    if (oref.startsWith("workspace:")) return h.workspaces[oref.slice("workspace:".length)] ?? null;
    return null;
};
vi.mock("@/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/app/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/store/services", () => services());
vi.mock("@/app/store/services", () => services());
vi.mock("@/app/window/window-close", () => ({
    requestRemoveWindow: async (w: unknown, onRemoving?: () => void) => {
        log(`requestRemoveWindow ${JSON.stringify(w)}`);
        onRemoving?.();
    },
    reopenClosedWindow: async (ws: string) => void log(`reopenClosedWindow ${ws}`),
    removeClosedWindow: async (ws: string) => void log(`removeClosedWindow ${ws}`),
}));
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({
    ...(await original<typeof import("@/app/element/ui/open-menu-at")>()),
    openMenuAt: (items: Array<{ label: string }>) => log(`menu ${items.map((i) => i.label).join(" | ")}`),
}));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: vi.fn() }));
vi.mock("@/app/store/window-opacity-store", () => ({
    liveWindowOpacity: (label: string) => h.live.get(label) ?? null,
    setWindowOpacityFor: (target: unknown, value: number, mode: string) => log(`setOpacity ${JSON.stringify(target)} ${value} ${mode}`),
}));
vi.mock("@/app/store/window-appearance", () => ({
    opacityFor: (meta?: Record<string, unknown>) => (meta?.["window:opacity"] as number | undefined) ?? 1,
    openNewWindowFromHere: async () => {
        log("openNewWindowFromHere");
        if (h.openNewFails) throw new Error("no new window");
    },
}));
vi.mock("@/app/store/config-signals", () => ({ settingsAtom: () => ({}) }));

import { InstancePanel } from "./InstancePanel";

const setEntries = (e: Array<{ label: string; windowId: string | null }>) => (globalThis as any).__setPanelEntries(e);
const flush = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
    for (let i = 0; i < 4; i++) await flush();
};
const pretty = (html: string): string => html.replace(/></g, ">\n<");
const out = () => h.log.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/InstancePanel.parts.${name}.txt`;

function panel(onClose: () => void = () => log("onClose")) {
    return render(() => <InstancePanel anchor={{ getBoundingClientRect: () => new DOMRect() } as any} onClose={onClose} />);
}
async function open(onClose?: () => void) {
    const view = panel(onClose);
    await settle();
    return view;
}
const rows = () => Array.from(document.querySelectorAll<HTMLElement>(".instance-panel-window-row:not(.instance-panel-window-row-closed)"));
const names = () => rows().map((r) => r.querySelector(".instance-panel-window-name")?.textContent ?? "(editing)");
const heading = () => document.querySelector<HTMLElement>(".instance-panel-workspace-title")!;

beforeEach(() => {
    h.log.length = 0;
    h.about = { version: "1.2.3" };
    h.dev = false;
    h.launcherActive = true;
    h.label = "main";
    h.snapshot = Promise.resolve([]);
    h.caps.clear();
    h.floating = [];
    h.windows = { win1: { meta: {}, workspaceid: "w1" }, win2: { meta: { "window:displayname": "Reviews" }, workspaceid: "w2" } };
    h.workspaces = { w1: { name: "Main Space" }, w2: { name: "Other Space" } };
    h.ownOid = null;
    h.live.clear();
    h.focusFails = false;
    h.openNewFails = false;
    h.updateMetaFails = false;
    h.renameFails = false;
    h.workspaceName = "Default Workspace";
    setEntries([
        { label: "main", windowId: "win1" },
        { label: "window-2", windowId: "win2" },
    ]);
    vi.spyOn(console, "error").mockImplementation((...a: unknown[]) => void log(`console.error ${a.map((x) => (x instanceof Error ? x.message : String(x))).join(" ")}`));
    vi.spyOn(console, "debug").mockImplementation((...a: unknown[]) => void log(`console.debug ${a.map(String).join(" ")}`));
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("opening the panel", () => {
    it("asks the host for the About details, then the window's label, then the closed windows", async () => {
        await open();
        // The double-click time the panel also asks for is not used: it is left out of this log.
        h.log.push(`rows: ${names().join(", ")}`);
        await expect(out()).toMatchFileSnapshot(snap("startup-active"));
    });

    it("when the launcher is silent, also reads the window list once and reconciles with it", async () => {
        h.launcherActive = false;
        h.snapshot = Promise.resolve([{ label: "main" }, { label: "window-2" }]);
        await open();
        await expect(out()).toMatchFileSnapshot(snap("startup-silent"));
    });

    it("drops that list if the launcher starts sending events while it is on its way", async () => {
        h.launcherActive = false;
        let arrive: (v: unknown[]) => void = () => {};
        h.snapshot = new Promise<unknown[]>((r) => (arrive = r));
        panel();
        await settle();
        h.launcherActive = true;
        arrive([{ label: "stale" }]);
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("startup-silent-flip"));
    });

    it("carries on when the host cannot say its label or list its windows", async () => {
        h.launcherActive = false;
        h.label = new Error("no label");
        h.snapshot = Promise.reject(new Error("no list"));
        await open();
        h.log.push(`rows: ${names().join(", ")}`);
        h.log.push(`this window's row marked: ${rows().some((r) => r.classList.contains("instance-panel-window-row-current"))}`);
        await expect(out()).toMatchFileSnapshot(snap("startup-failures"));
    });
});

describe("the About rows", () => {
    const header = () => pretty(document.querySelector(".instance-panel-header")!.outerHTML);
    const copyButtons = () => Array.from(document.querySelectorAll<HTMLButtonElement>(".instance-panel-copy"));

    it("shows what the host reports, with a copy button for the version, channel, commit and CEF", async () => {
        h.about = { version: "0.60.1", channel: "stable", buildLabel: "x", gitHash: "abc1234", buildTime: Date.UTC(2026, 10, 3, 9, 5), cefVersion: "152.0.1", platform: "darwin", arch: "arm64" };
        await open();
        h.log.push(header());
        for (const b of copyButtons()) {
            fireEvent.click(b);
            h.log.push(`clicked ${b.getAttribute("aria-label")}`);
        }
        await expect(out()).toMatchFileSnapshot(snap("about-full"));
    });

    it("marks a dev build, and leaves out what the host does not report", async () => {
        h.dev = true;
        await open();
        h.log.push(`no details: ${header()}`);
        cleanup();
        h.about = null;
        h.dev = false;
        await open();
        h.log.push(`host gave nothing: ${header()}`);
        cleanup();
        h.about = { version: "2", platform: "linux", buildTime: 0 };
        await open();
        h.log.push(`platform only, no build time: ${header()}`);
        cleanup();
        h.about = { version: "2", arch: "x64", buildTime: -5, channel: "dev" };
        await open();
        h.log.push(`arch only, a negative build time: ${header()}`);
        await expect(out()).toMatchFileSnapshot(snap("about-variants"));
    });
});

describe("a window's name and id", () => {
    it("takes the name from the window's own name, else its workspace's, else its place in the list", async () => {
        h.windows.win3 = { meta: {}, workspaceid: "w2" };
        h.windows.win4 = { meta: {} };
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
            { label: "window-3", windowId: "win3" },
            { label: "window-4", windowId: "win4" },
            { label: "window-5", windowId: null },
            { label: "window-6", windowId: "gone" },
        ]);
        await open();
        h.log.push(`names: ${names().join(" | ")}`);
        await expect(out()).toMatchFileSnapshot(snap("names"));
    });

    it("gives this window's own row the id the window object has while its entry has none yet", async () => {
        h.windows.own = { meta: { "window:displayname": "Mine" }, workspaceid: "w1" };
        h.ownOid = "own";
        setEntries([
            { label: "main", windowId: null },
            { label: "window-2", windowId: null },
        ]);
        await open();
        h.log.push(`names: ${names().join(" | ")}`);
        const remove = (i: number) => rows()[i].querySelector<HTMLButtonElement>(".instance-panel-row-action")!;
        fireEvent.click(remove(1));
        h.log.push("removed the other one without an id");
        fireEvent.click(remove(0));
        h.log.push("removed this one");
        fireEvent.dblClick(rows()[1]);
        h.log.push(`rename of the other opens: ${!!screen.queryByLabelText("Window name")}`);
        fireEvent.dblClick(rows()[0]);
        h.log.push(`rename of this one opens: ${!!screen.queryByLabelText("Window name")}`);
        await expect(out()).toMatchFileSnapshot(snap("own-id"));
    });

    it("cannot remove the only open window", async () => {
        setEntries([{ label: "main", windowId: "win1" }]);
        await open();
        fireEvent.click(rows()[0].querySelector<HTMLButtonElement>(".instance-panel-row-action")!);
        h.log.push("clicked remove");
        await expect(out()).toMatchFileSnapshot(snap("remove-only-window"));
    });
});

describe("a rename still open when the panel goes away", () => {
    const rename = (i: number, value: string) => {
        fireEvent.dblClick(rows()[i]);
        fireEvent.input(screen.getByLabelText("Window name"), { target: { value } });
    };

    it("is saved trimmed and capped, unless it is blank, its window has gone, or it has no id", async () => {
        const first = await open();
        rename(1, "  Saved on exit  ");
        first.unmount();
        h.log.push("-- blank");
        const second = await open();
        rename(1, "   ");
        second.unmount();
        h.log.push("-- a very long name");
        const third = await open();
        rename(1, "x".repeat(400));
        third.unmount();
        h.log.push("-- the window goes first");
        const fourth = await open();
        rename(1, "Late");
        setEntries([{ label: "main", windowId: "win1" }]);
        fourth.unmount();
        h.log.push("-- the window's id goes first");
        setEntries([{ label: "main", windowId: "win1" }, { label: "window-2", windowId: "win2" }]);
        const fifth = await open();
        rename(1, "No id");
        setEntries([{ label: "main", windowId: "win1" }, { label: "window-2", windowId: null }]);
        fifth.unmount();
        h.log.push("-- a refused save");
        setEntries([{ label: "main", windowId: "win1" }, { label: "window-2", windowId: "win2" }]);
        h.updateMetaFails = true;
        const sixth = await open();
        rename(1, "Refused");
        sixth.unmount();
        await settle();
        const logged = out().replace(/"x{20,}"/g, '"<400 x>"');
        await expect(logged).toMatchFileSnapshot(snap("rename-flush"));
    });

    it("saves the workspace's rename first, then the window's, when both are open", async () => {
        const view = await open();
        fireEvent.dblClick(heading());
        fireEvent.input(screen.getByLabelText("Workspace name"), { target: { value: "Renamed space" } });
        fireEvent.dblClick(rows()[1]);
        fireEvent.input(screen.getByLabelText("Window name"), { target: { value: "Renamed window" } });
        view.unmount();
        await expect(out()).toMatchFileSnapshot(snap("rename-flush-both"));
    });

    it("commits an edit of this window's name that no longer matches the row it was started on as nothing", async () => {
        await open();
        fireEvent.dblClick(rows()[1]);
        const input = screen.getByLabelText("Window name") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Stale" } });
        fireEvent.dblClick(rows()[0]);
        h.log.push(`after moving the edit to the other row: ${rows().map((r) => (r.querySelector("input") ? "editing" : "row")).join(", ")}`);
        fireEvent.blur(input);
        h.log.push("blurred the old input");
        await expect(out()).toMatchFileSnapshot(snap("rename-stale"));
    });
});

describe("when the panel is dismissed", () => {
    it("is closed through the anchored popover's dismissal", async () => {
        await open();
        h.dismiss();
        await expect(out()).toMatchFileSnapshot(snap("dismiss"));
    });
});

describe("a window that has no id anywhere", () => {
    it("shows its place in the list, cannot be renamed or removed, and says <null> in the console", async () => {
        h.ownOid = null;
        setEntries([
            { label: "main", windowId: null },
            { label: "window-2", windowId: "win2" },
        ]);
        await open();
        h.log.push(`names: ${names().join(" | ")}`);
        const mine = () => rows().find((r) => r.querySelector(".instance-panel-window-name")?.textContent === "Window 1")!;
        fireEvent.dblClick(mine());
        h.log.push(`rename opens: ${!!screen.queryByLabelText("Window name")}`);
        fireEvent.click(mine().querySelector<HTMLButtonElement>(".instance-panel-row-action")!);
        h.log.push("removed this one");
        await expect(out()).toMatchFileSnapshot(snap("own-id-missing"));
    });

    it("ends a rename that was open when the window lost its id, on commit", async () => {
        await open();
        fireEvent.dblClick(rows()[1]);
        fireEvent.input(screen.getByLabelText("Window name"), { target: { value: "Lost id" } });
        setEntries([{ label: "main", windowId: "win1" }, { label: "window-2", windowId: null }]);
        const input = screen.getByLabelText("Window name") as HTMLInputElement;
        fireEvent.blur(input);
        h.log.push(`editing after the commit: ${!!screen.queryByLabelText("Window name")}`);
        await expect(out()).toMatchFileSnapshot(snap("rename-no-id"));
    });
});

describe("the workspace heading's keys", () => {
    it("opens on F2 and a double-click once, commits on Enter or blur, cancels on Escape, and keeps the input's own keys", async () => {
        await open();
        const input = () => screen.queryByLabelText("Workspace name") as HTMLInputElement | null;
        fireEvent.keyDown(heading(), { key: "x" });
        h.log.push(`a other key: ${!!input()}`);
        fireEvent.keyDown(heading(), { key: "F2" });
        h.log.push(`F2 opens: ${input()?.value}`);
        fireEvent.input(input()!, { target: { value: "Typed" } });
        fireEvent.keyDown(heading(), { key: "F2" });
        fireEvent.dblClick(heading());
        h.log.push(`F2 and a double-click while editing keep the text: ${input()?.value}`);
        fireEvent.click(input()!);
        fireEvent.dblClick(input()!);
        fireEvent.keyDown(input()!, { key: "a" });
        h.log.push(`clicks and a key inside the input: ${input()?.value}`);
        fireEvent.keyDown(input()!, { key: "Escape" });
        h.log.push(`Escape: ${!!input()}`);
        fireEvent.dblClick(heading());
        fireEvent.input(input()!, { target: { value: "By Enter" } });
        fireEvent.keyDown(input()!, { key: "Enter" });
        h.log.push(`Enter: ${!!input()}`);
        fireEvent.dblClick(heading());
        fireEvent.input(input()!, { target: { value: "By blur" } });
        fireEvent.blur(input()!);
        h.log.push(`blur: ${!!input()}`);
        fireEvent.dblClick(heading());
        fireEvent.blur(input()!);
        h.log.push("unchanged name: no save");
        fireEvent.dblClick(heading());
        fireEvent.input(input()!, { target: { value: "  Default Workspace " } });
        fireEvent.keyDown(input()!, { key: "Enter" });
        h.log.push("same name with spaces: no save");
        h.renameFails = true;
        fireEvent.dblClick(heading());
        fireEvent.input(input()!, { target: { value: "Refused" } });
        fireEvent.keyDown(input()!, { key: "Enter" });
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("workspace-keys"));
    });
});

describe("a closed window's row", () => {
    it("reopens on Enter or Space or a click, not other keys, offers Remove on right-click, and its button reopens once", async () => {
        // The closed list comes from srv: a window the panel has not seen open is listed as closed at once.
        const closed = { workspaceid: "ws9", closed_at_ms: 1_700_000_000_000, name: "Old", tabs: 1, panes: 1, agents: [], running: 0 };
        const mod = await import("@/store/services");
        vi.spyOn(mod.WindowService, "ListClosedWindows").mockImplementation(async () => {
            log("ListClosedWindows");
            return [closed] as never;
        });
        await open();
        const row = () => document.querySelector<HTMLElement>(".instance-panel-window-row-closed")!;
        h.log.push(`closed row: ${row()?.getAttribute("role")} ${row()?.querySelector(".instance-panel-window-name")?.textContent}`);
        fireEvent.keyDown(row(), { key: "a" });
        h.log.push("other key");
        fireEvent.keyDown(row(), { key: "Enter" });
        fireEvent.keyDown(row(), { key: " " });
        fireEvent.click(row());
        h.log.push("-- context menu");
        fireEvent.contextMenu(row());
        h.log.push("-- the Reopen button");
        const button = row().querySelector<HTMLButtonElement>(".instance-panel-row-action")!;
        fireEvent.click(button);
        fireEvent.keyDown(button, { key: "Enter" });
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("closed-row"));
    });
});

describe("the floating panes", () => {
    const section = () => Array.from(document.querySelectorAll(".instance-panel-section")).find((s) => s.textContent?.includes("Floating panes"))!;
    const paneRows = () => Array.from(document.querySelectorAll<HTMLElement>(".instance-panel-pane-row"));

    it("counts them, names each, and says so when there are none", async () => {
        await open();
        h.log.push(pretty(section().outerHTML));
        cleanup();
        h.floating = [
            { label: "float-1", windowId: null },
            { label: "float-2", windowId: "win2", block_view: "agent" },
            { label: "float-3", windowId: "win1", block_view: "term" },
        ];
        await open();
        h.log.push(`title: ${section().querySelector(".instance-panel-section-title")!.textContent}`);
        h.log.push(`names: ${paneRows().map((r) => r.querySelector(".instance-panel-pane-name")!.textContent).join(" | ")}`);
        h.log.push(pretty(paneRows()[0].outerHTML));
        await expect(out()).toMatchFileSnapshot(snap("floating-list"));
    });

    it("focuses a pane on click, Enter or Space, and says so in the console when it cannot", async () => {
        h.floating = [{ label: "float-1", windowId: null }];
        await open();
        const row = paneRows()[0];
        fireEvent.click(row);
        fireEvent.keyDown(row, { key: "Enter" });
        fireEvent.keyDown(row, { key: " " });
        fireEvent.keyDown(row, { key: "a" });
        h.focusFails = true;
        fireEvent.click(row);
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("floating-focus"));
    });

    it("has an opacity control only on a host that can make windows see-through, session-only and keyed to the pane", async () => {
        h.floating = [{ label: "float-1", windowId: null }, { label: "float-2", windowId: null }];
        await open();
        h.log.push(`without the capability: ${paneRows()[0].querySelector(".instance-panel-opacity") ? "slider" : "no slider"}`);
        cleanup();
        h.caps.add("windowTransparency");
        h.live.set("float-1", 0.55);
        await open();
        const slider = paneRows()[0].querySelector<HTMLInputElement>(".instance-panel-opacity-slider")!;
        h.log.push(`value ${slider.value}, shown ${paneRows()[0].querySelector(".instance-panel-opacity-value")!.textContent}`);
        h.log.push(`a pane with no live value: ${paneRows()[1].querySelector<HTMLInputElement>(".instance-panel-opacity-slider")!.value}, shown ${paneRows()[1].querySelector(".instance-panel-opacity-value")!.textContent}`);
        fireEvent.input(slider, { target: { value: "0.5" } });
        fireEvent.change(slider, { target: { value: "0.7" } });
        fireEvent.click(slider);
        fireEvent.keyDown(slider, { key: "Enter" });
        h.log.push("a click and a key on the slider do not focus the pane");
        await expect(out()).toMatchFileSnapshot(snap("floating-opacity"));
    });
});

describe("focusing a window", () => {
    it("says so in the console when the host cannot focus it", async () => {
        await open();
        h.focusFails = true;
        fireEvent.click(rows()[1]);
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("focus-window-fails"));
    });
});

describe("the footer", () => {
    it("opens another window and closes the panel, or just closes, and the new-window button is only for a multi-window host", async () => {
        await open();
        h.log.push(`no multiWindow: ${document.querySelectorAll(".instance-panel-btn").length} button(s): ${Array.from(document.querySelectorAll(".instance-panel-btn")).map((b) => b.textContent).join(" | ")}`);
        fireEvent.click(screen.getByText("Close"));
        cleanup();
        h.caps.add("multiWindow");
        await open();
        h.log.push(`multiWindow: ${Array.from(document.querySelectorAll(".instance-panel-btn")).map((b) => b.textContent).join(" | ")}`);
        fireEvent.click(screen.getByText("+ Open another window"));
        await settle();
        h.openNewFails = true;
        fireEvent.click(screen.getByText("+ Open another window"));
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("footer"));
    });

    it("lays the sections out in order, with a divider between each", async () => {
        h.caps.add("multiWindow");
        h.floating = [{ label: "float-1", windowId: null }];
        await open();
        const body = document.querySelector('[data-testid="panel"]')!;
        const kids = Array.from(body.children).map((c) => `${c.tagName.toLowerCase()}.${c.className.split(" ").join(".")}`);
        h.log.push(kids.join("\n"));
        await expect(out()).toMatchFileSnapshot(snap("layout"));
    });
});
