// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A button locks at once when its click handler returns a promise, shows a spinner only if the action outlasts
 * `PENDING_SPINNER_DELAY_MS`, and unlocks when the promise settles however it ends
 * (docs/specs/SPEC_BUTTON_PRESSED_AND_PENDING_FEEDBACK_2026_11_20.md §4, §9).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Button, IconButton } from "./Button";
import { buttonTrace, clearButtonTrace, recordPress, TRACE_MIN_MS } from "./button-trace";
import { CloseButton } from "./CloseButton";
import { PENDING_SPINNER_DELAY_MS } from "./pending-press";

beforeEach(() => {
    vi.useFakeTimers();
    clearButtonTrace();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

const button = (c: HTMLElement) => c.querySelector("button")!;
const spinnerShown = (c: HTMLElement) => c.querySelector(".fa-spin") !== null;

/** A promise the test settles by hand. */
function deferred() {
    let resolve!: () => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<void>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

const flush = async () => {
    await Promise.resolve();
    await Promise.resolve();
};

describe("a button whose handler returns a promise", () => {
    it("is locked in the same call as the click, before anything awaits", () => {
        const d = deferred();
        const { container } = render(() => <Button onClick={() => d.promise}>Save</Button>);
        fireEvent.click(button(container));
        expect(button(container).getAttribute("aria-busy")).toBe("true");
        expect(button(container).getAttribute("data-pending")).toBe("true");
        expect(spinnerShown(container), "no spinner yet").toBe(false);
    });

    it("never shows a spinner for an action that settles inside the delay", async () => {
        const d = deferred();
        const { container } = render(() => <Button onClick={() => d.promise}>Save</Button>);
        fireEvent.click(button(container));
        vi.advanceTimersByTime(50);
        expect(spinnerShown(container)).toBe(false);
        d.resolve();
        await flush();
        vi.advanceTimersByTime(500);
        expect(spinnerShown(container)).toBe(false);
        expect(button(container).hasAttribute("aria-busy")).toBe(false);
    });

    it("shows the spinner at the delay, not before, over a label-only button", async () => {
        const d = deferred();
        const { container } = render(() => <Button onClick={() => d.promise}>Save</Button>);
        fireEvent.click(button(container));
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS - 1);
        expect(spinnerShown(container)).toBe(false);
        vi.advanceTimersByTime(1);
        expect(spinnerShown(container)).toBe(true);
        expect(button(container).classList.contains("ui-button--spin-over"), "over the label, which keeps its room").toBe(true);
        expect(button(container).textContent).toContain("Save");
        d.resolve();
        await flush();
        expect(spinnerShown(container)).toBe(false);
        expect(button(container).classList.contains("ui-button--spin-over")).toBe(false);
    });

    it("swaps the spinner for the icon of a button that has one", () => {
        const d = deferred();
        const { container } = render(() => <IconButton icon="floppy-disk" label="Save" tooltip={false} onClick={() => d.promise} />);
        expect(container.querySelector(".fa-floppy-disk")).not.toBeNull();
        fireEvent.click(button(container));
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS);
        expect(container.querySelector(".fa-floppy-disk"), "the glyph gives way").toBeNull();
        expect(container.querySelectorAll(".fa-spin")).toHaveLength(1);
        expect(button(container).classList.contains("ui-button--spin-over"), "no overlay: the spinner is in the glyph's place").toBe(false);
    });

    it("ignores a second click until the first settles, then works again", async () => {
        const first = deferred();
        const onClick = vi.fn(() => first.promise);
        const { container } = render(() => <Button onClick={onClick}>Save</Button>);
        fireEvent.click(button(container));
        fireEvent.click(button(container));
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(1);
        first.resolve();
        await flush();
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(2);
    });

    it("clears on a rejection and rethrows it, so the error reaches the unhandled-rejection path", async () => {
        const failure = new Error("boom");
        let fail!: (e: unknown) => void;
        // A thenable, so the test sees what the button does with the rejection handler it gave.
        const thenable = { then: (_ok: () => void, bad: (e: unknown) => void) => void (fail = bad) };
        const { container } = render(() => <Button onClick={() => thenable as unknown as Promise<void>}>Save</Button>);
        fireEvent.click(button(container));
        expect(button(container).getAttribute("aria-busy")).toBe("true");
        expect(() => fail(failure)).toThrow(failure);
        await flush();
        expect(button(container).hasAttribute("aria-busy"), "usable again").toBe(false);
        expect(buttonTrace(), "a fast failure is not slow").toHaveLength(0);
    });

    it("tells a screen reader once it has run a second, and logs a line", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const d = deferred();
        const { container } = render(() => (
            <Button aria-label="Install update" onClick={() => d.promise}>
                Install
            </Button>
        ));
        fireEvent.click(button(container));
        vi.advanceTimersByTime(999);
        expect(container.querySelector("[role=status]")).toBeNull();
        vi.advanceTimersByTime(1);
        expect(container.querySelector("[role=status]")).not.toBeNull();
        expect(warn).toHaveBeenCalledTimes(1);
        expect(String(warn.mock.calls[0][0])).toContain("Install update");
    });
});

describe("a button that does not go pending", () => {
    it("never goes pending for a handler that returns nothing", () => {
        const onClick = vi.fn();
        const { container } = render(() => <Button onClick={onClick}>Copy</Button>);
        fireEvent.click(button(container));
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(2);
        expect(button(container).hasAttribute("aria-busy")).toBe(false);
    });

    it("pending={false} runs every click and shows nothing, for a stepper or a person's answer", () => {
        const d = deferred();
        const onClick = vi.fn(() => d.promise);
        const { container } = render(() => (
            <Button pending={false} onClick={onClick}>
                +
            </Button>
        ));
        fireEvent.click(button(container));
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS * 3);
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(2);
        expect(button(container).hasAttribute("aria-busy")).toBe(false);
        expect(spinnerShown(container)).toBe(false);
    });

    it('pending="lock" locks but never shows a spinner', () => {
        const d = deferred();
        const onClick = vi.fn(() => d.promise);
        const { container } = render(() => (
            <Button pending="lock" onClick={onClick}>
                Save
            </Button>
        ));
        fireEvent.click(button(container));
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS * 3);
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(1);
        expect(button(container).getAttribute("aria-busy")).toBe("true");
        expect(spinnerShown(container)).toBe(false);
    });

    it("a toggle (pressed set) shows its state alone", () => {
        const d = deferred();
        const onClick = vi.fn(() => d.promise);
        const { container } = render(() => (
            <Button pressed={false} onClick={onClick}>
                Wrap
            </Button>
        ));
        fireEvent.click(button(container));
        fireEvent.click(button(container));
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS * 3);
        expect(onClick).toHaveBeenCalledTimes(2);
        expect(button(container).hasAttribute("aria-busy")).toBe(false);
        expect(spinnerShown(container)).toBe(false);
    });

    it("a caller's own busy still wins: a leading spinner and a disabled button", () => {
        const { container } = render(() => (
            <Button busy onClick={() => {}}>
                Save
            </Button>
        ));
        expect(button(container).disabled).toBe(true);
        expect(button(container).getAttribute("aria-busy")).toBe("true");
        expect(spinnerShown(container)).toBe(true);
        expect(button(container).classList.contains("ui-button--spin-over"), "its own look, not the overlay").toBe(false);
    });

    it("an IconButton takes busy", () => {
        const { container } = render(() => <IconButton icon="rotate" label="Refresh" tooltip={false} busy />);
        expect(button(container).disabled).toBe(true);
        expect(spinnerShown(container)).toBe(true);
    });

    it("runs Solid's [handler, data] form", () => {
        const onClick = vi.fn();
        const { container } = render(() => <Button onClick={[onClick, "row-7"]}>Open</Button>);
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledWith("row-7", expect.anything());
    });

    it("a disabled button runs nothing", () => {
        const onClick = vi.fn();
        const { container } = render(() => (
            <Button disabled onClick={onClick}>
                Save
            </Button>
        ));
        fireEvent.click(button(container));
        expect(onClick).not.toHaveBeenCalled();
    });
});

describe("the close control", () => {
    it("locks and swaps its ✕ for the spinner after the delay, and comes back", async () => {
        const d = deferred();
        const onClick = vi.fn(() => d.promise);
        const { container } = render(() => <CloseButton tooltip={false} onClick={onClick} />);
        fireEvent.click(button(container));
        fireEvent.click(button(container));
        expect(onClick).toHaveBeenCalledTimes(1);
        expect(container.querySelector(".fa-xmark")).not.toBeNull();
        vi.advanceTimersByTime(PENDING_SPINNER_DELAY_MS);
        expect(container.querySelector(".fa-xmark")).toBeNull();
        expect(spinnerShown(container)).toBe(true);
        d.resolve();
        await flush();
        expect(container.querySelector(".fa-xmark")).not.toBeNull();
    });
});

describe("the button trace", () => {
    it("keeps a press that stayed pending more than 300 ms, with its name and test id", () => {
        recordPress({ label: "Install", testId: "install-btn", ms: TRACE_MIN_MS + 1, failed: false });
        recordPress({ label: "Copy", ms: TRACE_MIN_MS, failed: false });
        expect(buttonTrace()).toEqual([{ label: "Install", testId: "install-btn", ms: TRACE_MIN_MS + 1, failed: false }]);
    });

    it("keeps the last 50, and is read as __buttonTrace()", () => {
        for (let i = 0; i < 60; i++) recordPress({ label: `b${i}`, ms: 400, failed: i % 2 === 0 });
        const read = (globalThis as { __buttonTrace?: () => { label: string }[] }).__buttonTrace!();
        expect(read).toHaveLength(50);
        expect(read[0].label).toBe("b10");
        expect(read.at(-1)!.label).toBe("b59");
    });
});
