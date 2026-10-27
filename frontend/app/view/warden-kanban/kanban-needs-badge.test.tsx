// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Needs-you badge on widget-bar entries that open the Swarm's Kanban (SPEC_WARDEN_KANBAN_2026_10_20.md §6.13). */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const { list, listeners } = vi.hoisted(() => ({
    list: vi.fn(),
    listeners: [] as (() => void)[],
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { KanbanListCommand: list } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_e: unknown, cb: () => void) => {
        listeners.push(cb);
        return () => listeners.splice(listeners.indexOf(cb), 1);
    },
}));

import { needsCountOf } from "./kanban-columns";
import { KanbanNeedsBadge, opensKanban } from "./kanban-needs-badge";

const t = (state: string, holder: unknown = null) => ({ id: state, state, holder, card: { num: 1 }, attempts: 1, title: "x" }) as never;
const board = (tasks: unknown[]) => ({ tasks, agents: [], stamp: 0, now: 0 });

afterEach(() => {
    cleanup();
    list.mockReset();
});

describe("the Kanban's Needs-you badge", () => {
    it("counts the tasks in Needs you, as the Swarm title does", () => {
        const tasks = [t("blocked"), t("failed"), t("claimed", { state: "waiting" }), t("claimed", { state: "working" }), t("review"), t("open")];
        expect(needsCountOf(tasks)).toBe(3);
    });

    it("shows on entries that open the Swarm's Kanban section, and only those", () => {
        expect(opensKanban({ blockdef: { meta: { view: "swarm", "swarm:section": "kanban" } } } as never)).toBe(true);
        expect(opensKanban({ blockdef: { meta: { view: "swarm" } } } as never)).toBe(false);
        expect(opensKanban({ blockdef: { meta: { view: "swarm", "swarm:section": "broadcast" } } } as never)).toBe(false);
        expect(opensKanban({ blockdef: { meta: { view: "warden", "warden:section": "vault" } } } as never)).toBe(false);
        // An entry from before the move, still naming Warden's Kanban, opens the Swarm's now.
        expect(opensKanban({ blockdef: { meta: { view: "warden", "warden:section": "kanban" } } } as never)).toBe(true);
        expect(opensKanban(undefined)).toBe(false);
    });

    it("shows the count, hides at zero, and follows the queue", async () => {
        list.mockResolvedValue(board([t("blocked"), t("failed")]));
        render(() => <KanbanNeedsBadge />);
        expect(await screen.findByRole("status")).toHaveTextContent("2");
        expect(screen.getByRole("status")).toHaveAttribute("aria-label", "2 need you");
        list.mockResolvedValue(board([t("open")]));
        listeners.forEach((cb) => cb());
        await waitFor(() => expect(screen.queryByRole("status")).toBeNull(), { timeout: 2000 });
    });
});
