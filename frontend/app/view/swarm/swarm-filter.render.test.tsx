// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm filter through its components: the bar and its count, the debounce, the pane's meta,
 * the empty result, the highlight, and what the selection and the fleet actions do with the
 * agents it hides. docs/specs/SPEC_SWARM_FILTER_BAR_2026_10_29.md §2, §6.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type Accessor } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({ ConfirmModal: () => null }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));

import { BroadcastDock } from "./swarm-broadcast";
import { Highlight, SwarmFilterBar, SwarmFilterEmpty } from "./swarm-filter-bar";
import { FILTER_APPLY_MS, FILTER_PERSIST_MS, actionableKeys, filterActive, hiddenByFilter, highlightWords, useSwarmFilter, type SwarmFilterState } from "./swarm-filter-state";
import { FleetToolbar } from "./swarm-fleet-toolbar";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";

const node = (blockId: string, agentName: string, line: string): AgentTreeNode =>
    ({
        blockId,
        agentName,
        line: { text: line, source: "generated" },
        currentTool: null,
        todoRows: [],
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
    }) as unknown as AgentTreeNode;

interface Rig {
    model: SwarmViewModel;
    filter: SwarmFilterState;
    setRows: (rows: [string, AgentTreeNode][]) => void;
    setSelected: (keys: string[]) => void;
    setMeta: ReturnType<typeof vi.fn>;
    container: HTMLElement;
}

/** The bar and the toolbar over one model, as the Swarm pane wires them. */
function rig(rows: [string, AgentTreeNode][], opts: { meta?: Record<string, unknown>; stopped?: { name: string }[] } = {}): Rig {
    const [keyed, setKeyed] = createSignal<readonly (readonly [string, AgentTreeNode])[]>(rows);
    const [selected, setSelectedSignal] = createSignal<Set<string>>(new Set());
    const setMeta = vi.fn(async () => {});
    const model = {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        fleetActionInFlightAtom: () => false,
        selectAll: vi.fn((ids: string[]) => setSelectedSignal(new Set(ids))),
        clearSelection: vi.fn(() => setSelectedSignal(new Set<string>())),
        setManySelected: vi.fn((ids: string[], on: boolean) =>
            setSelectedSignal((prev) => {
                const next = new Set(prev);
                for (const id of ids) on ? next.add(id) : next.delete(id);
                return next;
            })
        ),
        toggleSelected: () => {},
        isSelected: (id: string) => selected().has(id),
        fleetTargets: () => new Map(),
        broadcastToSelection: vi.fn(),
    } as unknown as SwarmViewModel;
    let filter!: SwarmFilterState;
    const ctx = { meta: () => opts.meta, setMeta } as never;
    const { container } = render(() => {
        filter = useSwarmFilter(model, ctx, { keyedRows: keyed, remote: () => [], stopped: () => (opts.stopped ?? []) as never });
        const shownSelectable = (): string[] => {
            const shown = filter.shown();
            const all = keyed().map(([k]) => k);
            return shown ? all.filter((k) => shown.has(k)) : all;
        };
        return (
            <>
                <SwarmFilterBar filter={filter} />
                <SwarmFilterEmpty filter={filter} />
                <FleetToolbar model={model} allBlockIds={shownSelectable} />
                <BroadcastDock model={model} agents={{ tree: () => keyed().map(([, n]) => n) }} />
            </>
        );
    });
    return { model, filter, setRows: (r) => setKeyed(r), setSelected: (k) => setSelectedSignal(new Set(k)), setMeta, container };
}

const ROWS: [string, AgentTreeNode][] = [
    ["a", node("a", "Lark", "Fixing the login redirect")],
    ["b", node("b", "Camper", "Writing the Swarm spec")],
    ["c", node("c", "Korp", "Login page in the browser")],
];
const type = (r: Rig, value: string) => fireEvent.input(r.container.querySelector<HTMLInputElement>("[data-testid=swarm-filter-input]")!, { target: { value } });
const count = (r: Rig) => r.container.querySelector("[data-testid=swarm-filter-count]")?.textContent ?? null;

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the filter bar", () => {
    it("shows no count with no filter; typing shows how many of the listed rows are shown after the debounce", () => {
        const r = rig(ROWS);
        expect(count(r)).toBeNull();
        type(r, "login");
        // The box has the text at once; the list has not applied it yet.
        expect((r.container.querySelector("[data-testid=swarm-filter-input]") as HTMLInputElement).value).toBe("login");
        expect(r.filter.shown()).toBeNull();
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect([...r.filter.shown()!]).toEqual(["a", "c"]);
        expect(count(r)).toBe("2 of 3");
    });

    it("a name: query counts only the rows whose name matches", () => {
        const r = rig(ROWS);
        type(r, "name:login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(r.filter.counts()).toEqual({ shown: 0, total: 3 });
        type(r, "work:login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(r.filter.counts()).toEqual({ shown: 2, total: 3 });
    });

    it("clearing applies at once, not after the debounce", () => {
        const r = rig(ROWS);
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        fireEvent.keyDown(r.container.querySelector("[data-testid=swarm-filter-input]")!, { key: "Escape" });
        expect(r.filter.text()).toBe("");
        expect(r.filter.shown()).toBeNull();
        expect(count(r)).toBeNull();
    });

    it("a second Escape, with the box empty, leaves it", () => {
        const r = rig(ROWS);
        const input = r.container.querySelector<HTMLInputElement>("[data-testid=swarm-filter-input]")!;
        input.focus();
        expect(document.activeElement).toBe(input);
        fireEvent.keyDown(input, { key: "Escape" });
        expect(document.activeElement).not.toBe(input);
    });

    it("says so when nothing matches, and Clear brings the list back", () => {
        const r = rig(ROWS);
        type(r, "zzz");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        const empty = r.container.querySelector("[data-testid=swarm-filter-empty]")!;
        expect(empty.textContent).toContain("No agents match “zzz”");
        fireEvent.click(empty.querySelector("button")!);
        expect(r.filter.text()).toBe("");
        expect(r.filter.shown()).toBeNull();
    });

    it("the stopped agents are filtered by name and counted", () => {
        const r = rig(ROWS, { stopped: [{ name: "Lark-2" }, { name: "Zed" }] });
        type(r, "lark");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(r.filter.stopped().map((a) => a.name)).toEqual(["Lark-2"]);
        expect(r.filter.counts()).toEqual({ shown: 2, total: 5 });
    });
});

describe("the filter in the pane's meta", () => {
    it("is written 500 ms after the last keystroke, and an empty box removes the key", () => {
        const r = rig(ROWS);
        type(r, "lar");
        type(r, "lark");
        vi.advanceTimersByTime(FILTER_PERSIST_MS - 1);
        expect(r.setMeta).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(r.setMeta).toHaveBeenCalledOnce();
        expect(r.setMeta).toHaveBeenCalledWith({ "swarm:filter": "lark" });
        r.filter.clear();
        vi.advanceTimersByTime(FILTER_PERSIST_MS);
        expect(r.setMeta).toHaveBeenLastCalledWith({ "swarm:filter": null });
    });

    it("a pane that comes back with a filter in its meta applies it at once, and shows the bar and count", () => {
        const r = rig(ROWS, { meta: { "swarm:filter": "korp" } });
        expect([...r.filter.shown()!]).toEqual(["c"]);
        expect(count(r)).toBe("1 of 3");
        expect((r.container.querySelector("[data-testid=swarm-filter-input]") as HTMLInputElement).value).toBe("korp");
    });
});

describe("a row never leaves under the cursor", () => {
    it("a row whose own update stops it matching stays until the filter text changes; one that starts matching appears", () => {
        const r = rig(ROWS);
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect([...r.filter.shown()!]).toEqual(["a", "c"]);
        // a's line changes, b's now matches.
        r.setRows([
            ["a", node("a", "Lark", "Reviewing a PR")],
            ["b", node("b", "Camper", "Login for the Swarm")],
            ["c", node("c", "Korp", "Login page in the browser")],
        ]);
        expect([...r.filter.shown()!].sort()).toEqual(["a", "b", "c"]);
        // The person edits the text: every row is judged again.
        type(r, "login ");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect([...r.filter.shown()!].sort()).toEqual(["a", "b", "c"]); // same words: nothing changed
        type(r, "login swarm");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect([...r.filter.shown()!]).toEqual(["b"]);
    });
});

describe("selection and the fleet actions with a filter on", () => {
    const toolbar = (r: Rig) => r.container.querySelector(".swarm-fleet-toolbar")!.textContent ?? "";

    it("Select all selects the shown agents only, and says how many; Select none leaves the hidden ones ticked", () => {
        const r = rig(ROWS);
        r.setSelected(["b"]); // b will be hidden
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(toolbar(r)).toContain("Select all 2 shown");
        fireEvent.click(r.container.querySelector(".swarm-fleet-btn")!);
        expect(r.model.selectedBlockIdsAtom()).toEqual(new Set(["a", "b", "c"]));
        expect(toolbar(r)).toContain("Select none 2 shown");
        fireEvent.click(r.container.querySelector(".swarm-fleet-btn")!);
        expect(r.model.selectedBlockIdsAtom()).toEqual(new Set(["b"]));
    });

    it("without a filter Select all is as it was: every listed agent, no count of shown", () => {
        const r = rig(ROWS);
        fireEvent.click(r.container.querySelector(".swarm-fleet-btn")!);
        expect(r.model.selectAll).toHaveBeenCalledWith(["a", "b", "c"]);
        expect(toolbar(r)).not.toContain("shown");
    });

    it("a selected agent the filter hides stays selected, is counted as hidden, and no action reaches it", () => {
        const r = rig(ROWS);
        r.setSelected(["a", "b"]);
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(hiddenByFilter(r.model)).toEqual(new Set(["b"]));
        expect(actionableKeys(r.model, r.model.selectedBlockIdsAtom())).toEqual(["a"]);
        expect(r.model.selectedBlockIdsAtom()).toEqual(new Set(["a", "b"]));
        expect(toolbar(r)).toContain("2 selected · 1 hidden by the filter, not included");
        // Broadcast says the same, and sends to the shown one only.
        expect(r.container.querySelector(".swarm-broadcast-actions")!.textContent).toContain("Send to 1");
        expect(r.container.querySelector(".swarm-broadcast-actions")!.textContent).toContain("1 hidden by the filter, not included");
    });

    it("clearing the filter puts the hidden agents back in reach", () => {
        const r = rig(ROWS);
        r.setSelected(["a", "b"]);
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        r.filter.clear();
        expect(hiddenByFilter(r.model).size).toBe(0);
        expect(actionableKeys(r.model, r.model.selectedBlockIdsAtom()).sort()).toEqual(["a", "b"]);
        expect(toolbar(r)).not.toContain("hidden by the filter");
    });
});

describe("the highlight", () => {
    it("marks what matched in a name or a line, and only in the field the query reads", () => {
        const r = rig(ROWS);
        type(r, "name:lar");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(filterActive(r.model)).toBe(true);
        expect(highlightWords(r.model, "name")).toEqual(["lar"]);
        expect(highlightWords(r.model, "work")).toEqual([]);
        type(r, "login");
        vi.advanceTimersByTime(FILTER_APPLY_MS);
        expect(highlightWords(r.model, "work")).toEqual(["login"]);
    });

    it("renders each match as a mark and the rest as plain text", () => {
        const { container } = render(() => <Highlight text="Fixing the Login redirect" words={["login"]} />);
        expect([...container.querySelectorAll("mark.swarm-match")].map((m) => m.textContent)).toEqual(["Login"]);
        expect(container.textContent).toBe("Fixing the Login redirect");
    });
});

describe("no filter, no cost", () => {
    it("reads no row while the query is empty: a row update does not re-run the match", () => {
        const [keyed, setKeyed] = createSignal<readonly (readonly [string, AgentTreeNode])[]>(ROWS);
        let reads = 0;
        const counted: Accessor<readonly (readonly [string, AgentTreeNode])[]> = () => (reads++, keyed());
        const model = {} as SwarmViewModel;
        render(() => {
            useSwarmFilter(model, { meta: () => undefined, setMeta: async () => {} } as never, { keyedRows: counted, remote: () => [], stopped: () => [] });
            return null;
        });
        const before = reads;
        setKeyed(ROWS.slice(0, 2));
        setKeyed(ROWS);
        expect(reads).toBe(before);
    });
});
