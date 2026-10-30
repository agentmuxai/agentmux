// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FilterChip, FilterChips, Tag, TagList, type TagSpec } from "./Tag";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
// The shared tooltip closes after a short grace period (its HIDE_DELAY_MS, 100 ms).
const afterHide = () => new Promise((resolve) => setTimeout(resolve, 160));

afterEach(cleanup);

describe("Tag", () => {
    it("is a neutral chip at the small size by default", () => {
        render(() => <Tag>Local</Tag>);
        const tag = screen.getByText("Local");
        for (const c of ["ui-tag", "ui-tag--chip", "ui-tag--neutral", "ui-tag--sm"]) {
            expect(tag.classList.contains(c), c).toBe(true);
        }
    });

    it("takes a tone, a size and a decorative icon", () => {
        const { container } = render(() => (
            <Tag tone="warn" size="xs" icon="circle-up">
                Upgrade
            </Tag>
        ));
        const tag = screen.getByText("Upgrade");
        expect(tag.classList.contains("ui-tag--warn")).toBe(true);
        expect(tag.classList.contains("ui-tag--xs")).toBe(true);
        const icon = container.querySelector("i");
        expect(icon?.className).toBe("fa-solid fa-circle-up");
        expect(icon?.getAttribute("aria-hidden")).toBe("true");
    });

    it("a ribbon sits on a corner (bottom right by default), at the small text size", () => {
        render(() => (
            <Tag kind="ribbon" tone="accent">
                Install
            </Tag>
        ));
        const tag = screen.getByText("Install");
        expect(tag.classList.contains("ui-tag--ribbon")).toBe(true);
        expect(tag.classList.contains("ui-tag--bottom-right")).toBe(true);
        expect(tag.classList.contains("ui-tag--xs")).toBe(true);
    });

    it("a count sits top right by default and takes another corner when told", () => {
        render(() => (
            <>
                <Tag kind="count">3</Tag>
                <Tag kind="count" corner="bottom-left">
                    4
                </Tag>
            </>
        ));
        expect(screen.getByText("3").classList.contains("ui-tag--top-right")).toBe(true);
        expect(screen.getByText("4").classList.contains("ui-tag--bottom-left")).toBe(true);
    });

    it("a chip with no detail is not focusable and keeps its native title", () => {
        render(() => <Tag title="Runs on this machine">Local</Tag>);
        const tag = screen.getByText("Local");
        expect(tag.getAttribute("tabindex")).toBeNull();
        expect(tag.getAttribute("title")).toBe("Runs on this machine");
    });

    describe("a chip with a detail", () => {
        it("takes focus and describes itself with hidden text", () => {
            render(() => <Tag detail="Checked 3 days ago">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            expect(tag.getAttribute("tabindex")).toBe("0");
            const id = tag.getAttribute("aria-describedby")!;
            const hidden = document.getElementById(id)!;
            expect(hidden.classList.contains("ui-tag-sr")).toBe(true);
            expect(hidden.textContent).toBe("Checked 3 days ago");
        });

        it("shows the detail while focused and closes it on blur", async () => {
            render(() => <Tag detail={<b>Checked 3 days ago</b>}>Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            expect(document.querySelector(".ui-tag-detail")).toBeNull();
            fireEvent.focus(tag);
            await tick();
            expect(document.querySelector(".ui-tag-detail")?.textContent).toBe("Checked 3 days ago");
            fireEvent.blur(tag);
            await afterHide();
            expect(document.querySelector(".ui-tag-detail")).toBeNull();
        });

        it("the hidden description is outside the chip, so the detail is not also its name", () => {
            render(() => <Tag detail="Checked 3 days ago">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            const hidden = document.getElementById(tag.getAttribute("aria-describedby")!)!;
            expect(tag.contains(hidden)).toBe(false);
            expect(tag.textContent).toBe("Free");
        });

        it("a tap (focus, then click) leaves the detail pinned open", async () => {
            render(() => <Tag detail="More">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            fireEvent.focus(tag);
            fireEvent.click(tag);
            await tick();
            expect(document.querySelector(".ui-tag-detail")).not.toBeNull();
        });

        it("hover, focus, leave, then blur closes the detail (a pinned tooltip does not keep a stale hover)", async () => {
            render(() => <Tag detail="More">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            const anchor = tag.closest(".ui-tag-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchor);
            fireEvent.focus(tag);
            await tick();
            fireEvent.mouseLeave(anchor);
            await tick();
            expect(document.querySelector(".ui-tag-detail")).not.toBeNull();
            fireEvent.blur(tag);
            await afterHide();
            expect(document.querySelector(".ui-tag-detail")).toBeNull();
        });

        it("Escape closes the detail even while the pointer is still over the chip", async () => {
            render(() => <Tag detail="More">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            const anchor = tag.closest(".ui-tag-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchor);
            fireEvent.focus(tag);
            await tick();
            expect(document.querySelector(".ui-tag-detail")).not.toBeNull();
            fireEvent.keyDown(tag, { key: "Escape" });
            await afterHide();
            expect(document.querySelector(".ui-tag-detail")).toBeNull();
        });

        it("a detail that is a stored JSX node stays in the hidden description after the tooltip opens and closes", async () => {
            const stored = (<b>Checked 3 days ago</b>) as unknown as HTMLElement;
            render(() => <Tag detail={stored}>Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            const hidden = document.getElementById(tag.getAttribute("aria-describedby")!)!;
            fireEvent.focus(tag);
            await tick();
            expect(document.querySelector(".ui-tag-detail")?.textContent).toBe("Checked 3 days ago");
            expect(hidden.textContent).toBe("Checked 3 days ago");
            fireEvent.blur(tag);
            await afterHide();
            expect(hidden.textContent).toBe("Checked 3 days ago");
        });

        it("a tap pins the detail open and Escape closes it", async () => {
            render(() => <Tag detail="More">Free</Tag>);
            const tag = screen.getByText("Free").closest(".ui-tag") as HTMLElement;
            fireEvent.click(tag);
            await tick();
            expect(document.querySelector(".ui-tag-detail")).not.toBeNull();
            fireEvent.keyDown(tag, { key: "Escape" });
            await afterHide();
            expect(document.querySelector(".ui-tag-detail")).toBeNull();
        });
    });
});

const spec = (id: string, extra: Partial<TagSpec> = {}): TagSpec => ({ id, label: id, ...extra });

const chipTexts = (container: HTMLElement) =>
    Array.from(container.querySelectorAll(".ui-tag-list > .ui-tag, .ui-tag-list .ui-tag-anchor > .ui-tag")).map((el) =>
        (el.firstChild as ChildNode | null)?.textContent ?? "",
    );

describe("TagList", () => {
    it("orders the chips by group, then as given", () => {
        const { container } = render(() => (
            <TagList
                max={9}
                tags={[
                    spec("count", { group: "fact" }),
                    spec("local", { group: "where" }),
                    spec("free", { group: "plan" }),
                    spec("broken", { group: "attention", tone: "error" }),
                    spec("other", { group: "fact" }),
                ]}
            />
        ));
        expect(chipTexts(container)).toEqual(["broken", "free", "local", "count", "other"]);
    });

    it("a tag with no group is a fact and sorts last", () => {
        const { container } = render(() => (
            <TagList max={9} tags={[spec("loose"), spec("local", { group: "where" })]} />
        ));
        expect(chipTexts(container)).toEqual(["local", "loose"]);
    });

    it("cuts to max from the end and folds the rest into +N, whose detail lists them", async () => {
        const { container } = render(() => (
            <TagList
                max={2}
                tags={[
                    spec("count", { group: "fact" }),
                    spec("local", { group: "where" }),
                    spec("free", { group: "plan" }),
                    spec("broken", { group: "attention", tone: "error" }),
                ]}
            />
        ));
        const texts = chipTexts(container);
        expect(texts).toEqual(["broken", "free", "+2"]);
        const more = screen.getByText("+2").closest(".ui-tag") as HTMLElement;
        fireEvent.focus(more);
        await tick();
        const items = Array.from(document.querySelectorAll(".ui-tag-detail .ui-tag-more li")).map((li) => li.textContent);
        expect(items).toEqual(["local", "count"]);
    });

    it("attention tags are cut last", () => {
        const { container } = render(() => (
            <TagList
                max={1}
                tags={[spec("free", { group: "plan" }), spec("broken", { group: "attention", tone: "error" })]}
            />
        ));
        expect(chipTexts(container)).toEqual(["broken", "+1"]);
    });

    it("shows three chips by default, with no +N when they fit", () => {
        const { container } = render(() => <TagList tags={[spec("a"), spec("b"), spec("c")]} />);
        expect(chipTexts(container)).toEqual(["a", "b", "c"]);
        expect(screen.queryByText(/^\+\d/)).toBeNull();
    });

    it("renders a ribbon apart on its corner and does not count it against max", () => {
        const { container } = render(() => (
            <TagList max={1} tags={[spec("Install", { kind: "ribbon", tone: "accent" }), spec("free", { group: "plan" })]} />
        ));
        expect(screen.getByText("Install").classList.contains("ui-tag--ribbon")).toBe(true);
        expect(chipTexts(container)).toEqual(["free"]);
        expect(screen.queryByText(/^\+\d/)).toBeNull();
    });

    it("renders nothing for no chips", () => {
        const { container } = render(() => <TagList tags={[]} />);
        expect(container.querySelector(".ui-tag-list")).toBeNull();
    });
});

describe("FilterChips", () => {
    function Harness(props: { initial?: string[]; onChange?: (v: string[]) => void }) {
        const [value, setValue] = createSignal<string[]>(props.initial ?? []);
        return (
            <FilterChips
                aria-label="Filter agents"
                value={value()}
                onChange={(next) => {
                    setValue(next);
                    props.onChange?.(next);
                }}
            >
                <FilterChip id="free" count={7}>
                    Free
                </FilterChip>
                <FilterChip id="local">Local</FilterChip>
            </FilterChips>
        );
    }

    it("is a labelled group of toggle buttons", () => {
        render(() => <Harness initial={["free"]} />);
        expect(screen.getByRole("group", { name: "Filter agents" })).toBeTruthy();
        const [free, local] = screen.getAllByRole("button");
        expect(free.getAttribute("aria-pressed")).toBe("true");
        expect(local.getAttribute("aria-pressed")).toBe("false");
    });

    it("any number can be on: a click toggles only its own chip", () => {
        const seen: string[][] = [];
        render(() => <Harness onChange={(v) => seen.push(v)} />);
        const [free, local] = screen.getAllByRole("button");
        fireEvent.click(free);
        fireEvent.click(local);
        fireEvent.click(free);
        expect(seen).toEqual([["free"], ["free", "local"], ["local"]]);
        expect(free.getAttribute("aria-pressed")).toBe("false");
        expect(local.getAttribute("aria-pressed")).toBe("true");
    });

    it("shows the count, and leaves it out when there is none", () => {
        const { container } = render(() => <Harness />);
        const counts = container.querySelectorAll(".ui-filter-chip-count");
        expect(counts.length).toBe(1);
        expect(counts[0].textContent).toBe("7");
    });

    it("a count of zero is shown", () => {
        const { container } = render(() => (
            <FilterChips aria-label="x" value={[]} onChange={() => {}}>
                <FilterChip id="a" count={0}>
                    A
                </FilterChip>
            </FilterChips>
        ));
        expect(container.querySelector(".ui-filter-chip-count")?.textContent).toBe("0");
    });

    it("a chip outside FilterChips is a mistake and says so", () => {
        const spy = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(() => render(() => <FilterChip id="a">A</FilterChip>)).toThrow(/inside FilterChips/);
        spy.mockRestore();
    });
});
