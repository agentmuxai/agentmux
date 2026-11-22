// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Pull requests not matched to a task": the list srv answers `pr_unmatched` with, linking one to a task by
// number, dismissing one, and the empty and closing states.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const { opened } = vi.hoisted(() => ({ opened: vi.fn(async () => undefined) }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: opened }));

import type { KanbanSectionState } from "./kanban-section-state";
import type { KanbanActRequest, KanbanActResult, KanbanUnmatchedPr } from "./kanban-types";
import { KanbanUnmatchedPrsPanel } from "./kanban-unmatched-prs";

function unmatched(number: number, over: Partial<KanbanUnmatchedPr> = {}): KanbanUnmatchedPr {
    return {
        repo: "agentmuxai/agentmux",
        number,
        url: `https://github.com/agentmuxai/agentmux/pull/${number}`,
        state: "open",
        agent_id: "camper",
        task_ref: "#99",
        why: "No task #99 is held by that agent",
        updated_at: 1,
        last_seen: 1,
        ...over,
    };
}

function mount(first: KanbanUnmatchedPr[]) {
    let rows = first;
    const act = vi.fn(async (req: KanbanActRequest): Promise<KanbanActResult | null> => {
        if (req.action === "pr_unmatched") return { ok: true, unmatched: rows };
        // A link or a dismiss takes the pull request off the list.
        rows = rows.filter((r) => `${r.repo}#${r.number}` !== req.pr);
        return { ok: true, changed: true };
    });
    const setManaging = vi.fn();
    const s = { model: { act }, setManaging } as unknown as KanbanSectionState;
    return { act, setManaging, ...render(() => <KanbanUnmatchedPrsPanel s={s} />) };
}

describe("the unmatched pull requests panel", () => {
    afterEach(cleanup);

    it("lists what srv could not match, with why, and opens one by its address", async () => {
        const { findByText, getByText } = mount([unmatched(12), unmatched(11, { state: "merged", why: "" })]);
        fireEvent.click(await findByText("agentmuxai/agentmux#12"));
        expect(opened).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/pull/12");
        expect(getByText("No task #99 is held by that agent")).toBeTruthy();
        expect(getByText("merged")).toBeTruthy();
    });

    it("links a pull request to the task number typed, and it leaves the list", async () => {
        const { act, container, findByText, queryByText } = mount([unmatched(12), unmatched(11)]);
        await findByText("agentmuxai/agentmux#12");
        const row = container.querySelector('[data-testid="kanban-unmatched-agentmuxai/agentmux-12"]')!;
        const link = [...row.querySelectorAll("button")].find((b) => b.textContent === "Link to task")!;
        expect(link.disabled).toBe(true);
        fireEvent.input(row.querySelector("input")!, { target: { value: "#48" } });
        fireEvent.submit(row.querySelector("form")!);
        expect(act).toHaveBeenCalledWith({ action: "pr_link", id: "48", pr: "agentmuxai/agentmux#12" });
        await vi.waitFor(() => expect(queryByText("agentmuxai/agentmux#12")).toBeNull());
        expect(queryByText("agentmuxai/agentmux#11")).toBeTruthy();
    });

    it("dismisses a pull request", async () => {
        const { act, findByText, queryByText, getAllByText } = mount([unmatched(12)]);
        await findByText("agentmuxai/agentmux#12");
        fireEvent.click(getAllByText("Dismiss")[0]);
        expect(act).toHaveBeenCalledWith({ action: "pr_dismiss", pr: "agentmuxai/agentmux#12" });
        await vi.waitFor(() => expect(queryByText("agentmuxai/agentmux#12")).toBeNull());
        expect(await findByText("Every pull request seen is on a task.")).toBeTruthy();
    });

    it("says so when there is none, and closes", async () => {
        const { findByText, setManaging, getByLabelText } = mount([]);
        expect(await findByText("Every pull request seen is on a task.")).toBeTruthy();
        fireEvent.click(getByLabelText("Close"));
        expect(setManaging).toHaveBeenCalledWith(false);
    });
});
