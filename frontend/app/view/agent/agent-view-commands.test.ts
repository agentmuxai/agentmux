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

    it("go away when unregistered", () => {
        off();
        expect(commandRegistry.get("agent:view:full")).toBeUndefined();
        off = registerAgentViewCommands();
    });
});
