// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// The palette's "Agent view" commands act on the focused agent pane, and only while there is one.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ focused: "" as string | undefined, controls: new Map<string, unknown>() }));

vi.mock("@/app/store/block-component-registry", () => ({ getFocusedBlockId: () => hub.focused }));
vi.mock("./agent-view-controls", () => ({ agentViewControls: (id: string | undefined) => (id ? hub.controls.get(id) : undefined) }));

import { commandRegistry, isCommandAvailable } from "@/app/store/command-registry";
import { registerAgentViewCommands } from "./agent-view-commands";
import { registerSearchControls } from "./search/search-bridge";

let off: () => void;
beforeEach(() => {
    off = registerAgentViewCommands();
});
afterEach(() => {
    off();
    hub.controls.clear();
});

const entry = (id: string) => commandRegistry.get(id)!;

describe("agent view palette commands", () => {
    it("are offered only while an agent pane is focused", () => {
        hub.focused = "term-1";
        expect(isCommandAvailable(entry("agent:view:standard"))).toBe(false);
        hub.controls.set("agent-1", { setPreset: vi.fn(), followDefault: vi.fn() });
        hub.focused = "agent-1";
        expect(isCommandAvailable(entry("agent:view:standard"))).toBe(true);
    });

    it("set the focused pane's preset, or make it follow the default", () => {
        const setPreset = vi.fn();
        const followDefault = vi.fn();
        hub.controls.set("agent-1", { setPreset, followDefault });
        hub.focused = "agent-1";
        void entry("agent:view:replies").execute();
        expect(setPreset).toHaveBeenCalledWith("replies");
        void entry("agent:view:follow-default").execute();
        expect(followDefault).toHaveBeenCalledTimes(1);
    });

    it("open the View popover on the focused pane", () => {
        const setPopoverOpen = vi.fn();
        hub.controls.set("agent-1", { setPopoverOpen });
        hub.focused = "agent-1";
        void entry("agent:view:open").execute();
        expect(setPopoverOpen).toHaveBeenCalledWith(true);
    });

    it("go away when unregistered", () => {
        off();
        expect(commandRegistry.get("agent:view:full")).toBeUndefined();
        off = registerAgentViewCommands();
    });
});

describe("Find in conversation (palette)", () => {
    it("is offered for a pane that has a find bar, and opens it", () => {
        const open = vi.fn();
        hub.focused = "agent-1";
        expect(isCommandAvailable(entry("agent:search:open"))).toBe(false);
        const unregister = registerSearchControls("agent-1", { visible: () => false, open, toggle: vi.fn() });
        expect(isCommandAvailable(entry("agent:search:open"))).toBe(true);
        void entry("agent:search:open").execute();
        expect(open).toHaveBeenCalledOnce();
        unregister();
        expect(isCommandAvailable(entry("agent:search:open"))).toBe(false);
    });
});
