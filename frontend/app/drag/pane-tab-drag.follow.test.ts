// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const watch = vi.hoisted(() => vi.fn());
vi.mock("./escape-watch", () => ({ watchEscapeDuringDrag: watch }));

import { endDrag } from "./drag-session";
import { startPaneTabDrag } from "./pane-tab-drag";

afterEach(() => {
    endDrag("cancel");
    watch.mockClear();
});

describe("a pane-tab drag's live follow", () => {
    it("follows a single tab that can tear off", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(watch).toHaveBeenCalledWith({ blockId: "b1" });
    });

    it("only watches Escape when several tabs are dragged together", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1", ["b1", "b2"]);
        expect(watch).toHaveBeenCalledWith(undefined);
    });

    it("only watches Escape when the strip names no window tab", () => {
        startPaneTabDrag("b1", "pane-1");
        expect(watch).toHaveBeenCalledWith(undefined);
    });
});
