// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pane tab on a remote machine names the remote in its pill (its Remotes
// nickname, else the connection) with the remote's icon, instead of a chip
// beside the tab strip that showed only while that tab was active.

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    connections: {} as Record<string, Record<string, unknown>>,
    status: {} as Record<string, { connected?: boolean; activeconnnum?: number }>,
}));
vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ connections: h.connections }) },
    getConnStatusAtom: (conn: string) => () => h.status[conn],
}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: vi.fn() }));

import { remoteOf, remoteTabIcon, remoteTabName } from "./pane-tab-remote";
import { termPaneTab } from "@/app/view/term/term-pane-tab";

const ctx = (meta: Record<string, unknown>, ordinal = 1) => ({ blockId: "b1", view: "term", meta, ordinal, liveViewModel: null }) as any;

beforeEach(() => {
    h.connections = {};
    h.status = {};
});

describe("remoteOf / remoteTabName", () => {
    it("is null for this computer, the connection otherwise", () => {
        expect(remoteOf({})).toBeNull();
        expect(remoteOf({ connection: "local" })).toBeNull();
        expect(remoteOf({ connection: "  " })).toBeNull();
        expect(remoteOf({ connection: "devbox" })).toBe("devbox");
    });

    it("names a remote by its Remotes nickname when it has one", () => {
        expect(remoteTabName("devbox")).toBe("devbox");
        h.connections = { devbox: { "display:name": "Gaming PC" } };
        expect(remoteTabName("devbox")).toBe("Gaming PC");
    });
});

describe("remoteTabIcon", () => {
    it("is a server (Linux for WSL), grey while not connected", () => {
        expect(remoteTabIcon("devbox")).toEqual({ kind: "fa", name: "server", color: "var(--grey-text-color)" });
        expect(remoteTabIcon("wsl://Ubuntu").name).toBe("brands@linux");
    });

    it("takes the remote's own colour while connected, else its connection colour", () => {
        h.status = { devbox: { connected: true, activeconnnum: 2 } };
        expect(remoteTabIcon("devbox").color).toBe("var(--conn-icon-color-2)");
        h.status = { devbox: { connected: true, activeconnnum: 8 } };
        expect(remoteTabIcon("devbox").color).toBe("var(--conn-icon-color-8)");
        h.connections = { devbox: { "display:color": "#ff8800" } };
        expect(remoteTabIcon("devbox").color).toBe("#ff8800");
    });
});

describe("terminal pill", () => {
    it("says the remote instead of Terminal N, with the remote's icon", () => {
        expect(termPaneTab.label!(ctx({ connection: "devbox" }))).toBe("devbox");
        expect(termPaneTab.icon!(ctx({ connection: "devbox" }))).toMatchObject({ kind: "fa", name: "server" });
    });

    it("a title the user gave it still wins, and a local terminal stays Terminal N", () => {
        expect(termPaneTab.label!(ctx({ connection: "devbox", "pane-title": "builds" }))).toBe("builds");
        expect(termPaneTab.label!(ctx({}, 3))).toBe("Terminal 3");
        expect(termPaneTab.icon!(ctx({}))).toBeUndefined();
    });
});
