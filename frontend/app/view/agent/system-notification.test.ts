// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createSystemNotifier } from "./system-notification";

describe("createSystemNotifier", () => {
    it("posts a new line, with a symbol for success and warning", () => {
        const dispatch = vi.fn();
        const post = createSystemNotifier(() => [], dispatch);
        post("Signed in", "success", "n1");
        post("Careful", "warning", "n2");
        post("Plain", "info", "n3");
        const contents = dispatch.mock.calls.map(([a]) => a.newNodes[0].content);
        expect(contents).toEqual(["✓ Signed in", "⚠ Careful", "Plain"]);
    });

    it("updates a line that is still in the pane, and posts it again when it is gone", () => {
        const dispatch = vi.fn();
        const inPane = createSystemNotifier(() => [{ id: "n1" } as any], dispatch);
        inPane("Signed in", "success", "n1", true);
        expect(dispatch.mock.calls[0][0]).toMatchObject({ newNodes: [], updatedNodes: [{ id: "n1" }] });
        const gone = createSystemNotifier(() => [], dispatch);
        gone("Signed in", "success", "n1", true);
        expect(dispatch.mock.calls[1][0]).toMatchObject({ newNodes: [{ id: "n1" }], updatedNodes: [] });
    });
});
