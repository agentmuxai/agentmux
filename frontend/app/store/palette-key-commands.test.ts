// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    view: undefined as string | undefined,
    dev: true,
    keyCommands: new Map<string, (e: unknown) => boolean>(),
    openBackdropPicker: vi.fn(),
    openMemory: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/app/store/block-component-registry", () => ({
    getFocusedBlockId: () => (h.view ? "b1" : undefined),
    getBlockComponentModel: () => (h.view ? { viewModel: { viewType: h.view } } : undefined),
}));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => h.dev }));
vi.mock("@/app/store/keymodel-dispatch", () => ({ keyCommands: h.keyCommands }));
vi.mock("@/app/backdrop/picker-state", () => ({ openBackdropPicker: h.openBackdropPicker }));
vi.mock("@/app/store/section-panes", () => ({ openMemory: h.openMemory }));

import { isCommandAvailable } from "./command-registry";
import { keyCommandEntries } from "./palette-key-commands";

const entry = (id: string) => keyCommandEntries().find((e) => e.id === id)!;
const shown = (id: string) => isCommandAvailable(entry(id));

describe("the global key commands", () => {
    beforeEach(() => {
        h.view = undefined;
        h.keyCommands.clear();
    });

    it("has a readable name, so a person who forgets the chord can find the command", () => {
        expect(entry("pane:new")).toMatchObject({ label: "New Pane", category: "Pane" });
        expect(entry("tab:rename").label).toBe("Rename Tab");
        expect(entry("pane:swap:left").label).toBe("Swap Pane Left");
        expect(entry("help:shortcuts").label).toBe("Keyboard shortcuts");
    });

    it("runs the handler its key runs", async () => {
        const handler = vi.fn(() => true);
        h.keyCommands.set("pane:new", handler);
        await entry("pane:new").execute();
        expect(handler).toHaveBeenCalledTimes(1);
    });

    it("shows the message box command only while an agent pane has focus", () => {
        expect(shown("agent:focusComposer")).toBe(false);
        h.view = "term";
        expect(shown("agent:focusComposer")).toBe(false);
        h.view = "agent";
        expect(shown("agent:focusComposer")).toBe(true);
    });

    it("shows the developer panels only in dev mode", () => {
        h.dev = false;
        expect(shown("dev:perfHud")).toBe(false);
        h.dev = true;
        expect(shown("dev:diagnostics")).toBe(true);
    });
});

describe("the pane-scoped commands", () => {
    beforeEach(() => {
        h.view = undefined;
        (window as unknown as { __agentmux_shortcuts: unknown }).__agentmux_shortcuts = { run: vi.fn(() => ({ ran: true })) };
    });

    it("come from the shortcut table and show only while a pane of their kind has focus", () => {
        expect(shown("files:back")).toBe(false);
        h.view = "files";
        expect(shown("files:back")).toBe(true);
        expect(shown("editor:save")).toBe(false);
        h.view = "editor";
        expect(shown("editor:save")).toBe(true);
        expect(entry("editor:save").category).toBe("Editor");
    });

    it("show document-tab commands in a pane that has document tabs, not in a Files pane", () => {
        h.view = "files";
        expect(shown("doctab:next")).toBe(false);
        h.view = "editor";
        expect(shown("doctab:next")).toBe(true);
        expect(entry("doctab:next").category).toBe("Documents");
    });

    it("show the terminal's own commands in a terminal", () => {
        h.view = "term";
        expect(shown("term:clear")).toBe(true);
        expect(entry("term:clear").category).toBe("Terminal");
        h.view = "agent";
        expect(shown("term:clear")).toBe(false);
    });

    it("run through the focused pane's runner, as MuxAPI's RunCommand does", async () => {
        h.view = "files";
        await entry("files:refresh").execute();
        const run = (window as unknown as { __agentmux_shortcuts: { run: ReturnType<typeof vi.fn> } }).__agentmux_shortcuts.run;
        expect(run).toHaveBeenCalledWith("files:refresh");
    });
});

describe("Backdrop… and Bundles", () => {
    beforeEach(() => {
        h.view = undefined;
        h.openBackdropPicker.mockClear();
        h.openMemory.mockClear();
    });

    it("offers Backdrop… only for a pane that takes one, and opens its picker", () => {
        h.view = "term";
        expect(shown("pane:backdrop")).toBe(false);
        h.view = "agent";
        expect(shown("pane:backdrop")).toBe(true);
        void entry("pane:backdrop").execute();
        expect(h.openBackdropPicker).toHaveBeenCalledWith("b1");
    });

    it("opens Memory on its Bundles section", () => {
        void entry("open:bundles").execute();
        expect(h.openMemory).toHaveBeenCalledWith("bundles");
    });
});
