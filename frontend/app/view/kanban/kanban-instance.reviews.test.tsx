// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A pane saved as the Reviews pane (the pane is part of the Kanban now): it opens the Kanban on the filter that
// showed what the Reviews pane did, and its task opens as a card.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";

const h = vi.hoisted(() => ({ props: null as null | Record<string, any> }));
vi.mock("./kanban-section", () => ({
    KanbanSection: (props: Record<string, any>) => {
        h.props = props;
        return null;
    },
}));

import { create } from "./kanban-instance";

function mount(meta: Record<string, unknown>) {
    const setMeta = vi.fn(async () => undefined);
    const ctx = { meta: () => meta, setMeta, visibility: () => "active" } as unknown as PaneTabHostContext;
    render(() => create(ctx).component({ ctx }));
    return { props: h.props!, setMeta };
}

afterEach(() => {
    cleanup();
    h.props = null;
});

describe("a pane saved as the Reviews pane", () => {
    it("opens the Kanban on cards with open pull requests", () => {
        expect(mount({ view: "reviews" }).props.waits).toBe("open");
    });

    it("opens the task it showed, and clears the request once it is shown", () => {
        const m = mount({ view: "reviews", "reviews:task": "work#12" });
        expect(m.props.openRequest).toBe("work#12");
        m.props.onOpenHandled();
        expect(m.setMeta).toHaveBeenCalledWith({ "kanban:open": null, "reviews:task": null });
    });

    it("keeps the filter the person chose, including none", () => {
        expect(mount({ view: "reviews", "kanban:waits": "" }).props.waits).toBe("");
        expect(mount({ view: "reviews", "kanban:waits": "author" }).props.waits).toBe("author");
    });
});

describe("the Kanban's own waits filter", () => {
    it("is none by default and keeps a known value, and a value it does not know is none", () => {
        expect(mount({ view: "kanban" }).props.waits).toBe("");
        expect(mount({ view: "kanban", "kanban:waits": "ci" }).props.waits).toBe("ci");
        expect(mount({ view: "kanban", "kanban:waits": "nonsense" }).props.waits).toBe("");
    });

    it("is written to the pane's meta when it changes", () => {
        const m = mount({ view: "kanban" });
        m.props.onWaits("reviewer");
        expect(m.setMeta).toHaveBeenCalledWith({ "kanban:waits": "reviewer" });
    });
});
