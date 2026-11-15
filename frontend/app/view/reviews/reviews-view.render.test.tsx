// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Reviews pane through its component: groups, selection, the narrow list-or-detail swap, empty and error states. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const opened = vi.hoisted(() => ({ links: [] as string[], tasks: [] as string[] }));
vi.mock("@/app/store/global", () => ({ openLink: (url: string) => opened.links.push(url) }));
vi.mock("@/app/util/open-kanban-task", () => ({ openKanbanTask: async (id: string) => void opened.tasks.push(id) }));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { ReviewRow } from "@/types/rpc/ReviewRow";
import type { ReviewsList } from "@/types/rpc/ReviewsList";
import type { ReviewsData } from "./reviews-store";
import { ReviewsView } from "./reviews-view";

afterEach(cleanup);

const NOW = 1_800_000_000_000;

function row(over: Partial<ReviewRow> & { number: number }): ReviewRow {
    return {
        branch: "",
        reviews: [],
        merge: "open",
        released_in: "",
        waits_on: "reviewer",
        why: "no_review",
        updated_at_ms: NOW - 60_000,
        ...over,
    } as ReviewRow;
}

const task = (num: number, title: string, state = "claimed") => ({ ref: `#${num}`, num, title, state });

const ROWS: ReviewRow[] = [
    row({
        number: 101,
        repo: "o/r",
        url: "https://github.com/o/r/pull/101",
        task: task(1, "Accept me", "review"),
        agent: "lazo",
        waits_on: "person",
        why: "awaiting_accept",
    }),
    row({
        number: 102,
        repo: "o/r",
        url: "https://github.com/o/r/pull/102",
        task: task(2, "Fix the thing"),
        agent: "camper",
        waits_on: "author",
        why: "checks_failing",
        checks: { state: "failing", at_ms: NOW },
        reviews: [{ by: "bot", state: "approved", at_ms: NOW - 120_000, stale: true }],
    }),
    row({ number: 103, repo_id: 9, agent: "posa" }),
];

function mount(
    opts: {
        rows?: ReviewRow[];
        status?: "loading" | "ready" | "unavailable";
        meta?: Record<string, unknown>;
        lastNotice?: number | null;
    } = {}
) {
    const [meta, setMetaSignal] = createSignal<Record<string, unknown>>(opts.meta ?? {});
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        setMetaSignal((m) => {
            const next = { ...m, ...patch };
            for (const k of Object.keys(next)) if (next[k] === null) delete next[k];
            return next;
        });
    });
    const list: ReviewsList = {
        rows: opts.rows ?? ROWS,
        as_of_ms: NOW,
        last_notice_at_ms: opts.lastNotice === undefined ? NOW : (opts.lastNotice ?? undefined),
    } as ReviewsList;
    const data: ReviewsData = {
        list: () => (opts.status === "loading" ? null : list),
        status: () => opts.status ?? "ready",
        refresh: vi.fn(),
        now: () => NOW,
    };
    const ctx = { meta, setMeta } as unknown as PaneTabHostContext;
    return { ...render(() => <ReviewsView data={data} ctx={ctx} />), setMeta };
}

/** A pane of this width: the view reads it once, when it mounts. */
function paneWidth(px: number) {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
        width: px,
        height: 600,
        top: 0,
        left: 0,
        right: px,
        bottom: 600,
        x: 0,
        y: 0,
        toJSON: () => ({}),
    });
}

beforeEach(() => {
    opened.links.length = 0;
    opened.tasks.length = 0;
    vi.stubGlobal(
        "ResizeObserver",
        class {
            observe() {}
            disconnect() {}
        }
    );
});
afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

describe("ReviewsView", () => {
    it("groups the rows with 'Needs a person' first, then the tasks, then the rows with no task", () => {
        const { container } = mount();
        const groups = [...container.querySelectorAll("[data-group]")].map((g) => g.getAttribute("data-group"));
        expect(groups).toEqual(["person", "task:2", "none"]);
    });

    it("shows a row's number, title and what it waits on, and a failing check and a stale review as words", () => {
        const { getAllByTestId } = mount();
        const text = getAllByTestId("reviews-row").map((r) => r.textContent ?? "");
        expect(text[0]).toContain("#101");
        expect(text[0]).toContain("Accept me");
        expect(text[0]).toContain("person");
        expect(text[1]).toContain("author");
        expect(text[1]).toContain("checks failing");
        expect(text[1]).toContain("stale review");
    });

    it("opens a detail beside the list on a click, with why, and opens the PR and the task", () => {
        const { getAllByTestId, getByTestId, getByText } = mount();
        fireEvent.click(getAllByTestId("reviews-row")[1]);
        const detail = getByTestId("reviews-detail");
        expect(detail.textContent).toContain("A check is failing");
        expect(detail.textContent).toContain("approved, older commit");
        expect(getByTestId("reviews-list")).toBeTruthy();
        fireEvent.click(getByText("Open on GitHub"));
        fireEvent.click(getByText("Open the task"));
        expect(opened.links).toEqual(["https://github.com/o/r/pull/102"]);
        expect(opened.tasks).toEqual(["2"]);
    });

    it("has nothing to open for a row with no address or task", () => {
        const { getAllByTestId, queryByText } = mount();
        fireEvent.click(getAllByTestId("reviews-row")[2]);
        expect(queryByText("Open on GitHub")).toBeNull();
        expect(queryByText("Open the task")).toBeNull();
    });

    it("filters by what is typed and by the waits-on switch, and says when nothing matches", () => {
        const { getByTestId, getAllByTestId, getByText, container } = mount();
        fireEvent.input(getByTestId("reviews-filter-input"), { target: { value: "camper" } });
        expect(getAllByTestId("reviews-row")).toHaveLength(1);
        fireEvent.input(getByTestId("reviews-filter-input"), { target: { value: "zzz" } });
        expect(getByText("Nothing matches")).toBeTruthy();
        fireEvent.input(getByTestId("reviews-filter-input"), { target: { value: "" } });
        fireEvent.click(getByText("Person"));
        expect(container.querySelectorAll("[data-testid=reviews-row]")).toHaveLength(1);
    });

    it("shows only the task it was opened for, and shows every task again on request", () => {
        const { getAllByTestId, getByText, setMeta } = mount({ meta: { "reviews:task": 2 } });
        expect(getAllByTestId("reviews-row")).toHaveLength(1);
        fireEvent.click(getByText("Show every task"));
        expect(setMeta).toHaveBeenCalledWith({ "reviews:task": null });
        expect(getAllByTestId("reviews-row")).toHaveLength(3);
    });

    it("says there is nothing yet, and what it is waiting for", () => {
        const { getByText } = mount({ rows: [] });
        expect(getByText("No pull requests on the board yet")).toBeTruthy();
    });

    it("says when srv cannot answer, and when it is still reading", () => {
        const down = mount({ status: "unavailable" });
        expect(down.getByText("Could not read the list")).toBeTruthy();
        cleanup();
        const loading = mount({ status: "loading" });
        expect(loading.getByText("Reading the pull requests")).toBeTruthy();
    });

    it("tells where the list comes from while no notice has arrived", () => {
        const { getByText } = mount({ lastNotice: null });
        expect(getByText(/pull requests your agents were notified about/)).toBeTruthy();
    });

    it("in a narrow pane shows the list, then the detail in its place with a way back", () => {
        paneWidth(320);
        const { getAllByTestId, getByTestId, queryByTestId } = mount();
        expect(queryByTestId("reviews-detail")).toBeNull();
        fireEvent.click(getAllByTestId("reviews-row")[0]);
        expect(getByTestId("reviews-detail")).toBeTruthy();
        expect(queryByTestId("reviews-list")).toBeNull();
        fireEvent.click(getByTestId("reviews-back"));
        expect(queryByTestId("reviews-detail")).toBeNull();
        expect(getByTestId("reviews-list")).toBeTruthy();
    });
});
