// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSessionStats, path by path: what the trigger button carries, where the panel is placed, each
 * row of the panel with its edges (the context reading with and without a window, the cost, turns and
 * time, the processed tokens and their cache share, the history, the archive date), when the Archive,
 * Restore and Export actions appear and how they behave while they work, and what closes the panel and
 * how its listeners come and go. Written ahead of splitting `AgentSessionStats`.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentSessionStats } from "./AgentSessionStats";

const archiveSession = vi.hoisted(() => vi.fn<(blockId: string) => Promise<void>>(async () => {}));
const exportSession = vi.hoisted(() => vi.fn<(blockId: string) => Promise<void>>(async () => {}));
const restoreSession = vi.hoisted(() => vi.fn<(blockId: string) => Promise<void>>(async () => {}));
const computeMenuPosition = vi.hoisted(() =>
    vi.fn<(opts: Record<string, unknown>) => Promise<{ style: Record<string, string> }>>(async () => ({
        style: { position: "fixed", left: "0px", top: "0px" },
    }))
);

vi.mock("../session-actions", async (importOriginal) => {
    const actual = await importOriginal<typeof import("../session-actions")>();
    return { ...actual, archiveSession, exportSession, restoreSession };
});
vi.mock("@/app/util/menu-position", () => ({ computeMenuPosition }));

const block = (meta: Record<string, unknown>) => () => ({ meta }) as never;

const baseProps = {
    blockId: "block-1",
    blockAtom: block({}),
    providerId: "claude",
    ctxClass: "ctx-warn",
    label: "104k / 200k",
};
type Props = Parameters<typeof AgentSessionStats>[0];

const trigger = () => screen.getByRole("button", { name: /104k \/ 200k|104k ctx|label here/ });
const openPanel = async () => {
    await userEvent.click(trigger());
    return await screen.findByRole("dialog", { name: "Session stats" });
};
const mount = async (over: Partial<Props> = {}) => {
    render(() => <AgentSessionStats {...baseProps} {...over} />);
    return await openPanel();
};
const rows = (panel: HTMLElement) =>
    Object.fromEntries(
        Array.from(panel.querySelectorAll(".agent-session-stats-row")).map((r) => [
            r.querySelector(".agent-session-stats-key")!.textContent,
            r.querySelector(".agent-session-stats-val")!.textContent,
        ])
    );
const valOf = (panel: HTMLElement, key: string) =>
    Array.from(panel.querySelectorAll(".agent-session-stats-row"))
        .find((r) => r.querySelector(".agent-session-stats-key")!.textContent === key)!
        .querySelector(".agent-session-stats-val") as HTMLElement;
const isOpen = () => screen.queryByRole("dialog", { name: "Session stats" }) !== null;
const NOTE = "Window reported by the CLI for claude-sonnet-5-5.";

function gate<T = void>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

beforeEach(() => {
    archiveSession.mockReset().mockResolvedValue(undefined);
    exportSession.mockReset().mockResolvedValue(undefined);
    restoreSession.mockReset().mockResolvedValue(undefined);
    computeMenuPosition.mockClear();
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("the trigger", () => {
    it("is the reading as a link-style strip button that says it opens a dialog, and carries the strip's band class and hover text", () => {
        render(() => <AgentSessionStats {...baseProps} title="Context: 52% full" />);
        const t = trigger();
        expect(t.textContent).toBe("104k / 200k");
        expect(t.className).toContain("ui-button--link");
        expect(t.className).toContain("agent-composer-strip-ctx");
        expect(t.className).toContain("agent-session-stats-trigger");
        expect(t.className).toContain("ctx-warn");
        expect(t).toHaveAttribute("data-strip-button");
        expect(t).toHaveAttribute("title", "Context: 52% full");
        expect(t).toHaveAttribute("aria-haspopup", "dialog");
        expect(t).toHaveAttribute("aria-expanded", "false");
        expect(isOpen()).toBe(false);
    });

    it("toggles the panel, saying whether it is expanded", async () => {
        render(() => <AgentSessionStats {...baseProps} />);
        await userEvent.click(trigger());
        expect(isOpen()).toBe(true);
        expect(trigger()).toHaveAttribute("aria-expanded", "true");
        await userEvent.click(trigger());
        await waitFor(() => expect(isOpen()).toBe(false));
        expect(trigger()).toHaveAttribute("aria-expanded", "false");
        await userEvent.click(trigger());
        expect(isOpen()).toBe(true);
    });

    it("follows the reading as it changes", async () => {
        const [label, setLabel] = createSignal("label here");
        render(() => <AgentSessionStats {...baseProps} label={label()} />);
        expect(screen.getByRole("button", { name: "label here" })).toBeInTheDocument();
        setLabel("104k ctx");
        expect(screen.getByRole("button", { name: "104k ctx" })).toBeInTheDocument();
    });
});

describe("the panel", () => {
    it("is a dialog called Session stats, with a Session heading, in the menu look, placed above its trigger", async () => {
        render(() => <AgentSessionStats {...baseProps} />);
        const rect = {
            left: 11,
            top: 22,
            right: 33,
            bottom: 44,
            width: 22,
            height: 22,
            x: 11,
            y: 22,
            toJSON: () => ({}),
        };
        vi.spyOn(trigger(), "getBoundingClientRect").mockReturnValue(rect as DOMRect);
        const panel = await openPanel();
        expect(panel.className).toContain("menu");
        expect(panel.className).toContain("agent-session-stats-panel");
        expect(panel.closest(".anchored-popover")!.className).toContain("anchored-popover--menu");
        expect(panel.querySelector(".agent-session-stats-section")!.textContent).toBe("Session");
        await waitFor(() => expect(computeMenuPosition).toHaveBeenCalled());
        const opts = computeMenuPosition.mock.calls[0][0];
        expect(opts.placement).toBe("top-start");
        expect(opts.anchor).toEqual(rect);
    });
});

describe("the context row", () => {
    it("reads tokens over window with the share, rounded to a whole percent and never over 100", async () => {
        let panel = await mount({ contextTokens: 104_400, contextWindow: 200_000 });
        expect(rows(panel).Context).toBe("104k / 200k (52%)");
        cleanup();
        panel = await mount({ contextTokens: 300_000, contextWindow: 200_000, label: "104k / 200k" });
        expect(rows(panel).Context).toBe("300k / 200k (100%)");
        cleanup();
        panel = await mount({ contextTokens: 1_000, contextWindow: 200_000 });
        expect(rows(panel).Context).toBe("1.0k / 200k (1%)");
    });

    it("carries the note as hover text on the reading, and shows it under the rows", async () => {
        const panel = await mount({ contextTokens: 104_000, contextWindow: 200_000, contextNote: NOTE });
        expect(valOf(panel, "Context").querySelector("[data-hover-tip]")!.getAttribute("data-hover-tip")).toBe(NOTE);
        expect(panel.querySelector(".agent-session-stats-note")!.textContent).toBe(NOTE);
        cleanup();
        const bare = await mount({ contextTokens: 104_000, contextWindow: 200_000 });
        expect(bare.querySelector(".agent-session-stats-note")).toBeNull();
        expect(valOf(bare, "Context").querySelector("[data-hover-tip]")).toBeNull();
    });

    it("says the window is unknown when it is missing or not above zero, with the same hover text and note", async () => {
        const one = await mount({ contextTokens: 1, contextWindow: undefined, label: "104k ctx" });
        expect(rows(one).Context).toBe("1 (window unknown)");
        cleanup();
        for (const contextWindow of [undefined, 0, -5]) {
            const panel = await mount({ contextTokens: 104_000, contextWindow, contextNote: NOTE, label: "104k ctx" });
            expect(rows(panel).Context, String(contextWindow)).toBe("104k (window unknown)");
            expect(valOf(panel, "Context").querySelector("[data-hover-tip]")!.getAttribute("data-hover-tip")).toBe(
                NOTE
            );
            expect(panel.querySelector(".agent-session-stats-note")!.textContent).toBe(NOTE);
            cleanup();
        }
    });

    it("shows no context row when there are no tokens in context, or only a note", async () => {
        for (const contextTokens of [undefined, null, 0, -1]) {
            const panel = await mount({ contextTokens, contextWindow: 200_000, contextNote: NOTE });
            expect(rows(panel).Context, String(contextTokens)).toBeUndefined();
            expect(panel.querySelector(".agent-session-stats-note")!.textContent).toBe(NOTE);
            cleanup();
        }
        const panel = await mount({ contextTokens: 0, contextWindow: undefined });
        expect(Object.keys(rows(panel))).toEqual([]);
    });

    it("follows the numbers as they change while open", async () => {
        const [tokens, setTokens] = createSignal<number | null>(104_000);
        render(() => <AgentSessionStats {...baseProps} contextTokens={tokens()} contextWindow={200_000} />);
        const panel = await openPanel();
        expect(rows(panel).Context).toBe("104k / 200k (52%)");
        setTokens(150_000);
        expect(rows(panel).Context).toBe("150k / 200k (75%)");
        setTokens(null);
        expect(rows(panel).Context).toBeUndefined();
    });
});

describe("the cost, turns and time", () => {
    const totals = (over: Record<string, unknown> = {}) => ({
        cost_usd: 2.41,
        num_turns: 37,
        duration_ms: 18 * 60_000,
        input_tokens: 1_000_000,
        output_tokens: 46_000,
        cache_read_input_tokens: 890_000,
        ...over,
    });

    it("shows nothing about totals until there are some", async () => {
        const panel = await mount({ sessionTotals: null });
        expect(rows(panel).Cost).toBeUndefined();
        expect(rows(panel).Processed).toBeUndefined();
        cleanup();
        const none = await mount({});
        expect(rows(none).Cost).toBeUndefined();
    });

    it("joins the cost, the turns and the time with dots", async () => {
        const panel = await mount({ sessionTotals: totals({ duration_ms: 90_000 }) });
        expect(rows(panel).Cost).toBe("$2.41 · 37 turns · 1m 30s");
    });

    it("leaves out the turns when there are none and the time when it is not known", async () => {
        let panel = await mount({ sessionTotals: totals({ num_turns: 0 }) });
        expect(rows(panel).Cost).toBe("$2.41 · 18m");
        cleanup();
        panel = await mount({ sessionTotals: totals({ num_turns: undefined, duration_ms: undefined }) });
        expect(rows(panel).Cost).toBe("$2.41");
        cleanup();
        panel = await mount({ sessionTotals: totals({ duration_ms: 0 }) });
        expect(rows(panel).Cost).toBe("$2.41 · 37 turns");
    });

    it("writes the cost to the cent, says less than a cent, and an em dash when there is none", async () => {
        const cases: [number | undefined, string][] = [
            [12.5, "$12.50"],
            [0.01, "$0.01"],
            [0.0099, "<$0.01"],
            [0, "<$0.01"],
            [0.1, "$0.10"],
            [undefined, "—"],
        ];
        for (const [cost, shown] of cases) {
            const panel = await mount({ sessionTotals: totals({ cost_usd: cost, num_turns: 0, duration_ms: 0 }) });
            expect(rows(panel).Cost, String(cost)).toBe(shown);
            cleanup();
        }
    });
});

describe("the processed tokens", () => {
    it("reads input and output, with the cache share of the input when it is known", async () => {
        const panel = await mount({
            sessionTotals: { input_tokens: 1_000_000, output_tokens: 46_000, cache_read_input_tokens: 890_000 },
        });
        expect(rows(panel).Processed).toBe("1.0m in (89% cached) · 46k out");
        expect(valOf(panel, "Processed").querySelector("[data-hover-tip]")!.getAttribute("data-hover-tip")).toMatch(
            /^Tokens sent and received across every API call in this pane's turns\. Each call re-sends the conversation, so input here counts the context once per call: it tracks cost, not how full the context is\.$/
        );
    });

    it("rounds the share to a whole percent and shows a zero share", async () => {
        let panel = await mount({
            sessionTotals: { input_tokens: 1000, output_tokens: 5, cache_read_input_tokens: 666 },
        });
        expect(rows(panel).Processed).toBe("1.0k in (67% cached) · 5 out");
        cleanup();
        panel = await mount({ sessionTotals: { input_tokens: 1000, output_tokens: 5, cache_read_input_tokens: 0 } });
        expect(rows(panel).Processed).toBe("1.0k in (0% cached) · 5 out");
    });

    it("says nothing of a cache share without input or without a cache reading", async () => {
        let panel = await mount({ sessionTotals: { input_tokens: 1000, output_tokens: 5 } });
        expect(rows(panel).Processed).toBe("1.0k in · 5 out");
        cleanup();
        panel = await mount({ sessionTotals: { input_tokens: 0, output_tokens: 5, cache_read_input_tokens: 10 } });
        expect(rows(panel).Processed).toBe("0 in · 5 out");
    });

    it("counts a missing side as zero", async () => {
        let panel = await mount({ sessionTotals: { output_tokens: 46_000 } });
        expect(rows(panel).Processed).toBe("0 in · 46k out");
        cleanup();
        panel = await mount({ sessionTotals: { input_tokens: 2_000 } });
        expect(rows(panel).Processed).toBe("2.0k in · 0 out");
    });
});

describe("the history and archive rows", () => {
    it("shows the lines with their exact count on hover and when the last activity was", async () => {
        const panel = await mount({
            blockAtom: block({
                "session:line_count": 12_483,
                "session:last_activity_ms": Date.now() - 3 * 60_000 - 5_000,
            }),
        });
        expect(rows(panel).History).toBe("12k lines · 3m ago");
        expect(valOf(panel, "History").querySelector("[data-hover-tip]")!.getAttribute("data-hover-tip")).toBe(
            "12,483 lines"
        );
    });

    it("shows a dash when there is no activity on record, and no row for an empty history", async () => {
        let panel = await mount({ blockAtom: block({ "session:line_count": 40 }) });
        expect(rows(panel).History).toBe("40 lines · —");
        cleanup();
        panel = await mount({ blockAtom: block({ "session:line_count": 0 }) });
        expect(rows(panel).History).toBeUndefined();
        cleanup();
        panel = await mount({ blockAtom: block({}) });
        expect(rows(panel).History).toBeUndefined();
    });

    it("shows when it was archived, in the reader's time, and not before", async () => {
        const at = 1_700_000_000_000;
        let panel = await mount({
            blockAtom: block({ "session:line_count": 10, "session:archived_at": at, "session:last_activity_ms": 5 }),
        });
        expect(rows(panel).Archived).toBe(new Date(at).toLocaleString());
        cleanup();
        panel = await mount({ blockAtom: block({ "session:line_count": 10 }) });
        expect(rows(panel).Archived).toBeUndefined();
    });
});

describe("the actions", () => {
    const withLines = block({ "session:line_count": 12_483 });
    const archived = block({ "session:line_count": 10, "session:archived_at": 1_700_000_000_000 });
    const names = () =>
        Array.from(document.querySelectorAll(".agent-session-stats-actions button")).map((b) => b.textContent);

    it("are offered for a Claude session with a history: Archive and Export", async () => {
        await mount({ blockAtom: withLines });
        expect(names()).toEqual(["Archive", "Export"]);
        expect(screen.getByRole("button", { name: "Archive" })).toHaveAttribute(
            "title",
            "Compress and archive this session's history to free disk space, then start fresh"
        );
        expect(screen.getByRole("button", { name: "Export" })).toHaveAttribute(
            "title",
            "Download this session's history as a .jsonl file"
        );
    });

    it("are Restore and Export once archived, even with no lines left", async () => {
        await mount({ blockAtom: block({ "session:archived_at": 1_700_000_000_000 }) });
        expect(names()).toEqual(["Restore", "Export"]);
        expect(screen.getByRole("button", { name: "Restore" })).toHaveAttribute(
            "title",
            "Restore this session's history from the archive"
        );
    });

    it("are left out for a session with nothing to manage, and for any other provider", async () => {
        await mount({ blockAtom: block({}) });
        expect(document.querySelector(".agent-session-stats-actions")).toBeNull();
        cleanup();
        await mount({ blockAtom: block({ "session:line_count": 0 }) });
        expect(document.querySelector(".agent-session-stats-actions")).toBeNull();
        cleanup();
        for (const providerId of ["codex", "gemini", "claude-x"]) {
            await mount({ providerId, blockAtom: archived });
            expect(document.querySelector(".agent-session-stats-actions"), providerId).toBeNull();
            cleanup();
        }
    });

    it("each call its own action with the block's id and nothing else", async () => {
        await mount({ blockAtom: withLines });
        await userEvent.click(screen.getByRole("button", { name: "Archive" }));
        await userEvent.click(screen.getByRole("button", { name: "Export" }));
        await waitFor(() => expect(exportSession).toHaveBeenCalledTimes(1));
        expect(archiveSession.mock.calls).toEqual([["block-1"]]);
        expect(exportSession.mock.calls).toEqual([["block-1"]]);
        expect(restoreSession).not.toHaveBeenCalled();
        cleanup();

        await mount({ blockAtom: archived, blockId: "block-2" });
        await userEvent.click(screen.getByRole("button", { name: "Restore" }));
        await waitFor(() => expect(restoreSession).toHaveBeenCalledTimes(1));
        expect(restoreSession.mock.calls).toEqual([["block-2"]]);
        expect(archiveSession).toHaveBeenCalledTimes(1);
        expect(exportSession).toHaveBeenCalledTimes(1);
    });

    it("show they are working and cannot be pressed again, each on its own, and come back when done", async () => {
        const a = gate();
        archiveSession.mockReturnValueOnce(a.promise);
        await mount({ blockAtom: withLines });
        const archive = () => screen.getByRole("button", { name: /Archiv/ });
        fireEvent.click(archive());
        await waitFor(() => expect(archive().textContent).toBe("Archiving…"));
        expect(archive()).toBeDisabled();
        expect(screen.getByRole("button", { name: "Export" })).not.toBeDisabled();
        fireEvent.click(archive());
        expect(archiveSession).toHaveBeenCalledTimes(1);
        a.resolve();
        await waitFor(() => expect(archive().textContent).toBe("Archive"));
        expect(archive()).not.toBeDisabled();
    });

    it("shows Restoring and Exporting the same way", async () => {
        const r = gate();
        const x = gate();
        restoreSession.mockReturnValueOnce(r.promise);
        exportSession.mockReturnValueOnce(x.promise);
        await mount({ blockAtom: archived });
        fireEvent.click(screen.getByRole("button", { name: "Restore" }));
        fireEvent.click(screen.getByRole("button", { name: "Export" }));
        await waitFor(() => expect(screen.getByRole("button", { name: /Restor/ }).textContent).toBe("Restoring…"));
        expect(screen.getByRole("button", { name: /Restor/ })).toBeDisabled();
        expect(screen.getByRole("button", { name: /Export/ }).textContent).toBe("Exporting…");
        expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
        fireEvent.click(screen.getByRole("button", { name: /Export/ }));
        fireEvent.click(screen.getByRole("button", { name: /Restor/ }));
        expect(restoreSession).toHaveBeenCalledTimes(1);
        expect(exportSession).toHaveBeenCalledTimes(1);
        r.resolve();
        x.resolve();
        await waitFor(() => expect(screen.getByRole("button", { name: "Restore" })).not.toBeDisabled());
        expect(screen.getByRole("button", { name: "Export" })).not.toBeDisabled();
    });

    it("log what failed, by name, and let the person try again, with the panel still open", async () => {
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        const boom = new Error("disk full");
        archiveSession.mockRejectedValueOnce(boom);
        exportSession.mockRejectedValueOnce(boom);
        await mount({ blockAtom: withLines });
        await userEvent.click(screen.getByRole("button", { name: "Archive" }));
        await waitFor(() => expect(err).toHaveBeenCalledWith("session:archive failed:", boom));
        await waitFor(() => expect(screen.getByRole("button", { name: "Archive" })).not.toBeDisabled());
        await userEvent.click(screen.getByRole("button", { name: "Export" }));
        await waitFor(() => expect(err).toHaveBeenCalledWith("session:export failed:", boom));
        await waitFor(() => expect(screen.getByRole("button", { name: "Export" })).not.toBeDisabled());
        expect(isOpen()).toBe(true);
        await userEvent.click(screen.getByRole("button", { name: "Archive" }));
        await waitFor(() => expect(archiveSession).toHaveBeenCalledTimes(2));
        cleanup();

        restoreSession.mockRejectedValueOnce(boom);
        await mount({ blockAtom: archived });
        await userEvent.click(screen.getByRole("button", { name: "Restore" }));
        await waitFor(() => expect(err).toHaveBeenCalledWith("session:restore failed:", boom));
        await waitFor(() => expect(screen.getByRole("button", { name: "Restore" })).not.toBeDisabled());
    });
});

describe("closing", () => {
    const outside = () => {
        const el = document.createElement("button");
        el.textContent = "elsewhere";
        document.body.appendChild(el);
        return el;
    };

    it("closes on a press outside it, and not on a press inside it or on its trigger", async () => {
        await mount({ blockAtom: block({ "session:line_count": 5 }) });
        fireEvent.mouseDown(screen.getByRole("dialog"));
        fireEvent.mouseDown(screen.getByRole("dialog").querySelector(".agent-session-stats-section")!);
        fireEvent.mouseDown(trigger());
        expect(isOpen()).toBe(true);
        const el = outside();
        fireEvent.mouseDown(el);
        await waitFor(() => expect(isOpen()).toBe(false));
        el.remove();
    });

    it("does not treat a plain click outside as a press", async () => {
        await mount({});
        const el = outside();
        fireEvent.click(el);
        expect(isOpen()).toBe(true);
        el.remove();
    });

    it("closes on Escape, hands the focus back to the trigger and keeps the key from anything else", async () => {
        await mount({ blockAtom: block({ "session:line_count": 5 }) });
        screen.getByRole("button", { name: "Export" }).focus();
        expect(document.activeElement).not.toBe(trigger());
        const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
        document.body.dispatchEvent(escape);
        expect(escape.defaultPrevented).toBe(true);
        await waitFor(() => expect(isOpen()).toBe(false));
        expect(document.activeElement).toBe(trigger());
    });

    it("ignores other keys", async () => {
        await mount({});
        const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
        document.body.dispatchEvent(enter);
        expect(enter.defaultPrevented).toBe(false);
        expect(isOpen()).toBe(true);
    });

    it("hears Escape before whatever has the focus does", async () => {
        await mount({});
        const input = document.createElement("input");
        input.addEventListener("keydown", (e) => e.stopPropagation());
        document.body.appendChild(input);
        fireEvent.keyDown(input, { key: "Escape" });
        await waitFor(() => expect(isOpen()).toBe(false));
        input.remove();
    });

    it("closes when the focus goes elsewhere, and stays when it moves between the trigger and the panel", async () => {
        await mount({ blockAtom: block({ "session:line_count": 5 }) });
        screen.getByRole("button", { name: "Export" }).focus();
        expect(isOpen()).toBe(true);
        trigger().focus();
        expect(isOpen()).toBe(true);
        const el = outside();
        el.focus();
        await waitFor(() => expect(isOpen()).toBe(false));
        el.remove();
    });

    it("listens only while open: nothing before it opens, all three gone once it closes, and again on the next open", async () => {
        const add = vi.spyOn(document, "addEventListener");
        const remove = vi.spyOn(document, "removeEventListener");
        render(() => <AgentSessionStats {...baseProps} />);
        const ours = (spy: typeof add) =>
            spy.mock.calls.filter((c) => ["mousedown", "keydown", "focusin"].includes(c[0] as string));
        expect(ours(add)).toHaveLength(0);
        await userEvent.click(trigger());
        const added = ours(add);
        expect(added.map((c) => c[0]).sort()).toEqual(["focusin", "keydown", "mousedown"]);
        expect(added.find((c) => c[0] === "keydown")![2]).toBe(true);
        await userEvent.click(trigger());
        await waitFor(() => expect(isOpen()).toBe(false));
        const removed = ours(remove);
        expect(removed.map((c) => c[0]).sort()).toEqual(["focusin", "keydown", "mousedown"]);
        expect(removed.find((c) => c[0] === "keydown")![2]).toBe(true);
        for (const type of ["mousedown", "keydown", "focusin"]) {
            const was = added.find((c) => c[0] === type)![1];
            expect(removed.find((c) => c[0] === type)![1]).toBe(was);
        }
        // Closed: a stray Escape is nobody's business.
        const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
        document.body.dispatchEvent(escape);
        expect(escape.defaultPrevented).toBe(false);
        await userEvent.click(trigger());
        expect(isOpen()).toBe(true);
        document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
        await waitFor(() => expect(isOpen()).toBe(false));
    });
});
