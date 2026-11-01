// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const objects = vi.hoisted(() => new Map<string, unknown>());
const pushed = vi.hoisted(() => [] as NotificationType[]);
const actions = vi.hoisted(() => new Map<string, () => void>());
const redock = vi.hoisted(() => vi.fn(async () => {}));
const revealed = vi.hoisted(() => vi.fn(async () => ({ ok: true })));

vi.mock("@/app/notification/usenotification", () => ({ setNotificationAction: (k: string, fn: () => void) => void actions.set(k, fn) }));
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "my-tab" },
    pushNotification: (n: NotificationType) => void pushed.push(n),
    workspace: () => ({ oid: "ws-here" }),
}));
vi.mock("@/app/store/services", () => ({
    ClientService: { GetClientData: async () => ({ windowids: ["w-here", "w2"] }) },
    WorkspaceService: { RedockFloatingPane: redock },
}));
const goResults = vi.hoisted(() => [] as unknown[]);
vi.mock("@/app/util/reveal-block", () => ({
    loadObject: async (otype: string, oid: string) => objects.get(`${otype}:${oid}`) ?? null,
    revealBlock: revealed,
    revealInOtherWindow: vi.fn(async () => goResults.shift()),
}));

import { findPaneInOtherWindow, offerPaneElsewhere } from "./reveal-app-scope";
import type { PaneElsewhere } from "./reveal-or-open";

const put = (otype: string, oid: string, value: object) => objects.set(`${otype}:${oid}`, { oid, ...value });

beforeEach(() => {
    objects.clear();
    pushed.length = 0;
    actions.clear();
    redock.mockClear();
    revealed.mockClear();
    goResults.length = 0;
    put("window", "w-here", { workspaceid: "ws-here" });
    put("window", "w2", { workspaceid: "ws2" });
    put("workspace", "ws2", { tabids: ["t1", "t2"] });
    put("tab", "t1", { blockids: ["b-other"] });
    put("tab", "t2", { blockids: ["b-agent"] });
    put("block", "b-other", { meta: { view: "agent", agentId: "someone-else", agentName: "Else" } });
    put("block", "b-agent", { meta: { view: "agent", agentId: "parlo", agentName: "Parlo" } });
});

describe("findPaneInOtherWindow", () => {
    it("finds the pane of the same identity in another window, in any of its tabs", async () => {
        const found = await findPaneInOtherWindow({ meta: { view: "agent", agentId: "parlo" } } as BlockDef);
        expect(found).toEqual({ blockId: "b-agent", windowId: "w2", tabId: "t2", workspaceId: "ws2", name: "Parlo", liveProcess: true });
    });

    it("does not take another agent's pane for it", async () => {
        expect(await findPaneInOtherWindow({ meta: { view: "agent", agentId: "nobody" } } as BlockDef)).toBeNull();
    });

    it("looks nowhere for a pane that has no identity", async () => {
        expect(await findPaneInOtherWindow({ meta: { view: "agent" } } as BlockDef)).toBeNull();
        expect(await findPaneInOtherWindow({ meta: { view: "settings" } } as BlockDef)).toBeNull();
    });

    it("leaves this window's own panes to the window search", async () => {
        put("window", "w2", { workspaceid: "ws-here" });
        expect(await findPaneInOtherWindow({ meta: { view: "agent", agentId: "parlo" } } as BlockDef)).toBeNull();
    });

    it("finds an Editor document by its file, and says it is not a live process", async () => {
        put("block", "b-doc", { meta: { view: "editor", file: "C:/notes.md", title: "notes.md" } });
        put("tab", "t2", { blockids: ["b-doc"] });
        const found = await findPaneInOtherWindow({ meta: { view: "editor", file: "C:/notes.md" } } as BlockDef);
        expect(found).toMatchObject({ blockId: "b-doc", name: "notes.md", liveProcess: false });
    });
});

describe("offerPaneElsewhere", () => {
    const pane = (over: Partial<PaneElsewhere> = {}): PaneElsewhere => ({
        blockId: "b1",
        windowId: "w2",
        tabId: "t2",
        workspaceId: "ws2",
        name: "notes.md",
        liveProcess: false,
        ...over,
    });

    it("offers Go there and Show here for a pane that can move, and a notice that stays", () => {
        offerPaneElsewhere(pane(), "refused");
        expect(pushed).toHaveLength(1);
        expect(pushed[0]).toMatchObject({ title: "notes.md is open in another window", persistent: true });
        expect(pushed[0].actions?.map((a) => a.label)).toEqual(["Go there", "Show here"]);
    });

    it("offers only Go there for a pane with a live process: it is never moved", () => {
        offerPaneElsewhere(pane({ liveProcess: true, name: "Parlo" }), "unknown");
        expect(pushed[0].actions?.map((a) => a.label)).toEqual(["Go there"]);
        expect([...actions.keys()].some((k) => k.endsWith(":show"))).toBe(false);
    });

    it("moves the pane into this window's shown tab and shows it there", async () => {
        offerPaneElsewhere(pane(), "refused");
        const show = [...actions].find(([k]) => k.endsWith(":show"))![1];
        show();
        await vi.waitFor(() => expect(revealed).toHaveBeenCalledWith("b1", { source: "widget", localOnly: true }));
        expect(redock).toHaveBeenCalledWith("b1", "t2", "ws2", "my-tab", "ws-here");
    });
});

describe("the notice's buttons", () => {
    const pane: PaneElsewhere = { blockId: "b1", windowId: "w2", tabId: "t2", workspaceId: "ws2", name: "notes.md", liveProcess: false };
    const behind = { found: true, raised: true, restored: false, moved: false, foreground: false };
    const run = async (key: string) => {
        actions.get(`reveal-elsewhere:b1:${key}`)!();
        await new Promise((r) => setTimeout(r, 0));
    };

    it("Go there offers the notice again when the window was kept behind once more", async () => {
        goResults.push({ outcome: "revealed", raise: behind });
        offerPaneElsewhere(pane, "refused");
        pushed.length = 0;
        await run("go");
        expect(pushed).toHaveLength(1);
        expect(pushed[0].title).toBe("notes.md is open in another window");
    });

    it("Go there says nothing more when the window came forward", async () => {
        goResults.push({ outcome: "revealed", raise: { ...behind, foreground: true } });
        offerPaneElsewhere(pane, "refused");
        pushed.length = 0;
        await run("go");
        expect(pushed).toHaveLength(0);
    });

    it("Show here that fails says why and puts the notice back", async () => {
        redock.mockRejectedValueOnce(new Error("no such tab"));
        offerPaneElsewhere(pane, "refused");
        pushed.length = 0;
        await run("show");
        expect(pushed.map((n) => n.title)).toEqual(["Couldn't move notes.md here", "notes.md is open in another window"]);
        expect(pushed[0].message).toBe("no such tab");
    });
});
