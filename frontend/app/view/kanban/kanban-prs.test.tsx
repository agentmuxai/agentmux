// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A card's pull requests: the chip's count and
// worst state, the "n of n merged, waiting on" line, and the drawer's list.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const { opened } = vi.hoisted(() => ({ opened: vi.fn(async () => undefined) }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: opened }));

import { DrawerPrs, prChip, prsOf, prStateWords, PrChip, WaitingOnLine } from "./kanban-prs";
import type { KanbanCardFields, KanbanCardPr, KanbanPrRollup } from "./kanban-types";

const NOW = 10_000_000;

function pr(number: number, over: Partial<KanbanCardPr> = {}): KanbanCardPr {
    return {
        repo: "agentmuxai/agentmux",
        number,
        url: `https://github.com/agentmuxai/agentmux/pull/${number}`,
        state: "open",
        review: "none",
        checks: "none",
        opened_at: NOW - 3_600_000,
        updated_at: NOW - 60_000,
        source: "holder",
        ...over,
    } as KanbanCardPr;
}

function rollup(prs: KanbanCardPr[], over: Partial<KanbanPrRollup> = {}): KanbanPrRollup {
    const merged = prs.filter((p) => p.state === "merged").length;
    const closed = prs.filter((p) => p.state === "closed").length;
    const open = prs.filter((p) => p.state === "open").length;
    return {
        total: prs.length,
        open,
        merged,
        closed_unmerged: closed,
        state: open > 0 ? "open" : merged > 0 ? "merged" : prs.length ? "closed" : "none",
        follow_up_open: false,
        all_merged_waiting: false,
        ...over,
    };
}

function card(prs: KanbanCardPr[] | undefined, roll?: Partial<KanbanPrRollup>): KanbanCardFields {
    return {
        num: 1,
        repo: "agentmuxai/agentmux",
        branch: "",
        pr_url: prs?.[0]?.url ?? "",
        review_required: false,
        checklist: [],
        question: "",
        note: "",
        requested_by: "",
        requested_by_kind: "",
        request_text: "",
        filed_by: "",
        updated_at: 1,
        prs,
        pr_rollup: prs ? rollup(prs, roll) : undefined,
    } as KanbanCardFields;
}

afterEach(() => {
    cleanup();
    opened.mockClear();
});

describe("a card's pull request chip", () => {
    it("is absent for no pull request, one pull request, and an older srv with no list", () => {
        expect(prChip(card(undefined))).toBeNull();
        expect(prChip(card([]))).toBeNull();
        expect(prChip(card([pr(1)]))).toBeNull();
    });

    it("counts the merged ones of the whole, and is ok when every one merged", () => {
        const two = [pr(1, { state: "merged" }), pr(2)];
        expect(prChip(card(two))).toMatchObject({ label: "1 of 2 merged", tone: "accent" });
        const done = [pr(1, { state: "merged" }), pr(2, { state: "merged" }), pr(3, { state: "merged" })];
        expect(prChip(card(done))).toMatchObject({ label: "3 of 3 merged", tone: "ok" });
    });

    it("says how many there are when none has merged, and notes a follow-up", () => {
        expect(prChip(card([pr(1), pr(2)]))?.label).toBe("2 PRs");
        const prs = [pr(1, { state: "merged" }), pr(2)];
        expect(prChip(card(prs, { follow_up_open: true }))?.label).toBe("1 of 2 merged · follow-up open");
    });

    it("warns when an open one has failing checks or asked for changes, and names each in its title", () => {
        const prs = [pr(1, { state: "merged" }), pr(2, { checks: "failing" }), pr(3, { review: "changes_requested" })];
        const chip = prChip(card(prs))!;
        expect(chip.tone).toBe("warn");
        expect(chip.title.split("\n")).toEqual([
            "agentmuxai/agentmux#1: merged",
            "agentmuxai/agentmux#2: open, checks failing",
            "agentmuxai/agentmux#3: open, changes requested",
        ]);
    });

    it("does not count a pull request that was taken off the card", () => {
        const prs = [pr(1, { state: "merged" }), pr(2, { unlinked: true })];
        expect(prsOf(card(prs))).toHaveLength(1);
    });

    it("draws as a pill with the label", () => {
        const { container } = render(() => <PrChip card={card([pr(1, { state: "merged" }), pr(2)])} />);
        expect(container.textContent).toContain("1 of 2 merged");
    });
});

describe("the words for a pull request's state", () => {
    it.each([
        [{ state: "merged" }, "merged"],
        [{ state: "closed" }, "closed without merging"],
        [{ state: "open", review: "approved", checks: "passing" }, "open, approved"],
        [{ state: "open", checks: "pending" }, "open, checks running"],
    ])("%j reads %s", (over, words) => {
        expect(prStateWords(pr(1, over as Partial<KanbanCardPr>))).toBe(words);
    });
});

describe("the line for a held task whose pull requests all merged", () => {
    const merged = [pr(1, { state: "merged" }), pr(2, { state: "merged" }), pr(3, { state: "merged" })];

    it("reads 'n of n merged, waiting on' and the holder", () => {
        const { container } = render(() => (
            <WaitingOnLine card={card(merged, { all_merged_waiting: true })} holder={() => <b>Opaz</b>} />
        ));
        expect(container.textContent?.replace(/\s+/g, " ").trim()).toBe("3 of 3 merged, waiting on Opaz");
    });

    it("is absent until srv says the task is waiting", () => {
        const { container } = render(() => <WaitingOnLine card={card(merged)} holder={() => <b>Opaz</b>} />);
        expect(container.textContent).toBe("");
    });
});

describe("the drawer's list", () => {
    it("lists every pull request in the order srv sent, each with its state, and opens one by its address", () => {
        const prs = [pr(12, { state: "merged", merged_at: NOW - 120_000 }), pr(11)];
        const { container } = render(() => <DrawerPrs card={card(prs)} now={NOW} />);
        const rows = [...container.querySelectorAll("li")];
        expect(rows.map((r) => r.querySelector("a")?.textContent)).toEqual(["agentmuxai/agentmux#12", "agentmuxai/agentmux#11"]);
        expect(rows[0].textContent).toContain("merged");
        expect(rows[1].textContent).toContain("open");
        fireEvent.click(rows[1].querySelector("a")!);
        expect(opened).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/pull/11");
    });

    it("is absent for one pull request, where the drawer's own button stays", () => {
        const { container } = render(() => <DrawerPrs card={card([pr(1)])} now={NOW} />);
        expect(container.querySelector("ul")).toBeNull();
    });
});
