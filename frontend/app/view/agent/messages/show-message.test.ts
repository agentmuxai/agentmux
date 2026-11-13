// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { DocumentNode } from "@/app/agent-model/node-types";
import { describe, expect, it, vi } from "vitest";
import type { MessagesPanelControls } from "./messages-state";
import { showMessage, type ShowMessageDeps } from "./show-message";

const jekt = (id: string) => ({ id, type: "jekt_message", direction: "incoming" }) as unknown as DocumentNode;

function setup(opts: { panelOnly?: string[]; rows?: string[]; open?: boolean; noPanel?: boolean } = {}) {
    const nodes = [jekt("sensitive"), jekt("quiet")];
    const panelOnly = new Set(opts.panelOnly ?? []);
    const rows = new Set(opts.rows ?? ["sensitive"]);
    let open = opts.open ?? false;
    const panel = {
        state: () => ({ enabled: true, open, seen: 0 }),
        patch: vi.fn((c: { open?: boolean }) => void (c.open !== undefined && (open = c.open))),
        showInPanel: vi.fn(),
        hidden: (n: DocumentNode) => panelOnly.has(n.id),
    } as unknown as MessagesPanelControls;
    const deps = {
        panel: opts.noPanel ? undefined : panel,
        nodes: () => nodes,
        hasRow: (id: string) => rows.has(id),
        jump: vi.fn(),
        mark: vi.fn(),
    } satisfies ShowMessageDeps;
    return { deps, panel, isOpen: () => open };
}

describe("showMessage: what 'Show message' does, by where the message is", () => {
    it("scrolls to the row and marks it when the conversation has one, so a row already on screen still answers", () => {
        const t = setup();
        expect(showMessage("sensitive", t.deps)).toBe("row");
        expect(t.deps.jump).toHaveBeenCalledWith("sensitive");
        expect(t.deps.mark).toHaveBeenCalledWith("sensitive");
        expect(t.panel.showInPanel).not.toHaveBeenCalled();
        expect(t.panel.patch).not.toHaveBeenCalled();
    });

    it("opens the panel on a message the conversation has no row for, instead of jumping to nothing", () => {
        const t = setup({ rows: [] });
        expect(showMessage("sensitive", t.deps)).toBe("panel");
        expect(t.panel.showInPanel).toHaveBeenCalledWith("sensitive");
        expect(t.deps.jump).not.toHaveBeenCalled();
        expect(t.deps.mark).not.toHaveBeenCalled();
    });

    it("opens the panel on a message only the panel holds, even where a row would be found", () => {
        const t = setup({ panelOnly: ["quiet"], rows: ["quiet"] });
        expect(showMessage("quiet", t.deps)).toBe("panel");
        expect(t.panel.showInPanel).toHaveBeenCalledWith("quiet");
        expect(t.deps.jump).not.toHaveBeenCalled();
    });

    it("closes the panel before it jumps, because the panel lies over the conversation", () => {
        const t = setup({ open: true });
        expect(showMessage("sensitive", t.deps)).toBe("row");
        expect(t.panel.patch).toHaveBeenCalledWith({ open: false });
        expect(t.isOpen()).toBe(false);
        expect(t.deps.jump).toHaveBeenCalledWith("sensitive");
        expect(t.deps.mark).toHaveBeenCalledWith("sensitive");
    });

    it("still jumps and marks where the pane has no panel", () => {
        const t = setup({ noPanel: true });
        expect(showMessage("sensitive", t.deps)).toBe("row");
        expect(t.deps.jump).toHaveBeenCalledWith("sensitive");
        expect(t.deps.mark).toHaveBeenCalledWith("sensitive");
    });
});
