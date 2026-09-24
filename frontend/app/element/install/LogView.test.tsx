// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { render, cleanup, waitFor, fireEvent } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));

import { ContextMenuModel } from "@/app/store/contextmenu";

import type { LogLine } from "./install-session";
import { LOG_ROW_PX, LogView, type LogViewApi } from "./LogView";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const E = String.fromCharCode(27);
const makeLines = (n: number): LogLine[] => Array.from({ length: n }, (_, i) => ({ text: `line ${i}`, tone: "normal" }));

function renderLog(initial: LogLine[], trimmed = 0) {
    const [lines, setLines] = createSignal<readonly LogLine[]>(initial);
    let api!: LogViewApi;
    const utils = render(() => (
        <LogView lines={lines} trimmedLines={() => trimmed} copyAllText={() => "ALL"} apiRef={(a) => (api = a)} />
    ));
    const el = utils.container.querySelector(".install-log") as HTMLElement;
    const rows = () => Array.from(utils.container.querySelectorAll(".install-log-line")).map((r) => r.textContent);
    return { ...utils, el, rows, api: () => api, setLines };
}

describe("LogView", () => {
    it("renders only a window of rows for a long log, sized to the full length", () => {
        const { el, rows } = renderLog(makeLines(20_000));
        const spacer = el.querySelector(".install-log-spacer") as HTMLElement;
        expect(spacer.style.height).toBe(`${20_000 * LOG_ROW_PX}px`);
        expect(rows().length).toBeLessThan(100);
        expect(rows()[0]).toBe("line 0");
    });

    it("renders the rows around a line it was asked to scroll to", async () => {
        const { el, rows, api } = renderLog(makeLines(20_000));
        api().scrollToLine(12_345);
        expect(el.scrollTop).toBe((12_345 - 2) * LOG_ROW_PX);
        await waitFor(() => expect(rows()).toContain("line 12345"));
        expect(rows()).not.toContain("line 0");
    });

    it("tags rows with their tone and renders ANSI colours as spans", () => {
        const { container } = renderLog([
            { text: "npm error code ENOTFOUND", tone: "error" },
            { text: `${E}[32mok${E}[0m plain`, tone: "normal" },
        ]);
        const lines = container.querySelectorAll<HTMLElement>(".install-log-line");
        expect(lines[0].dataset.tone).toBe("error");
        expect(lines[1].textContent).toBe("ok plain");
        expect(lines[1].querySelector(".log-fg-green")?.textContent).toBe("ok");
    });

    it("shows a marker row when earlier lines were trimmed, and offsets scrolling past it", () => {
        const { rows, el, api } = renderLog(makeLines(10), 1_000);
        expect(rows()[0]).toBe("… 1000 earlier lines trimmed");
        expect(rows()[1]).toBe("line 0");
        api().scrollToLine(5);
        // Line 5 is row 6 once the marker is counted.
        expect(el.scrollTop).toBe((6 - 2) * LOG_ROW_PX);
    });

    it("offers Copy and Copy All on right-click", () => {
        const { el } = renderLog(makeLines(3));
        fireEvent.contextMenu(el);
        const items = vi.mocked(ContextMenuModel.showContextMenu).mock.calls[0][0] as Array<{ label: string; enabled: boolean }>;
        expect(items.map((i) => [i.label, i.enabled])).toEqual([
            ["Copy", false],
            ["Copy All", true],
        ]);
    });

    describe("sticks to the bottom (ported from SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §3-§4)", () => {
        // jsdom has no layout engine; stub the box sizes.
        const stub = (scrollHeight: number, clientHeight: number) => ({
            scroll: vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(scrollHeight),
            client: vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(clientHeight),
        });
        afterEach(() => vi.restoreAllMocks());

        it("follows new lines while at the bottom", async () => {
            const { scroll } = stub(100, 50);
            const { el, setLines } = renderLog(makeLines(1));
            el.scrollTop = 50;
            scroll.mockReturnValue(120);
            setLines(makeLines(2));
            await vi.waitFor(() => expect(el.scrollTop).toBe(120));
        });

        it("stops following once the user scrolls away", async () => {
            stub(200, 50);
            const { el, setLines } = renderLog(makeLines(1));
            el.scrollTop = 0;
            fireEvent.scroll(el);
            setLines(makeLines(2));
            await new Promise((r) => setTimeout(r, 20));
            expect(el.scrollTop).toBe(0);
        });

        it("doesn't yank back if the user scrolls away between a scheduled frame and its execution (codex P2, PR #3165)", () => {
            const frames: FrameRequestCallback[] = [];
            const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
                frames.push(cb);
                return frames.length;
            });
            try {
                stub(200, 50);
                const { el, setLines } = renderLog(makeLines(1));
                el.scrollTop = 200;
                setLines(makeLines(2)); // schedules a follow frame
                el.scrollTop = 0;
                fireEvent.scroll(el); // user escapes before it runs
                frames[frames.length - 1](0);
                expect(el.scrollTop).toBe(0);
            } finally {
                raf.mockRestore();
            }
        });

        it("resumes following when the user scrolls back within 40px of the bottom", async () => {
            const { scroll } = stub(200, 50);
            const { el, setLines } = renderLog(makeLines(1));
            el.scrollTop = 0;
            fireEvent.scroll(el);
            el.scrollTop = 155;
            fireEvent.scroll(el);
            scroll.mockReturnValue(240);
            setLines(makeLines(2));
            await vi.waitFor(() => expect(el.scrollTop).toBe(240));
        });

        it("re-syncs to the bottom on refresh (Details opened) while following", async () => {
            stub(300, 50);
            const { el, api } = renderLog(makeLines(5));
            el.scrollTop = 0; // stale position left while collapsed
            api().refresh();
            await vi.waitFor(() => expect(el.scrollTop).toBe(300));
        });

        it("keeps following when its own scroll event arrives after more lines grew the log (seen live)", async () => {
            // A programmatic scroll's event is dispatched a frame later; by
            // then a burst of npm lines has grown scrollHeight, so the
            // event looks like the user scrolling away from the bottom.
            const { scroll } = stub(200, 50);
            const { el, setLines } = renderLog(makeLines(1));
            setLines(makeLines(2));
            await vi.waitFor(() => expect(el.scrollTop).toBe(200)); // followed
            scroll.mockReturnValue(400); // more lines landed before the event
            fireEvent.scroll(el); // our own scroll's late event
            setLines(makeLines(3));
            await vi.waitFor(() => expect(el.scrollTop).toBe(400));
        });

        it("starts following again when the log is cleared for a new run", async () => {
            const { scroll } = stub(200, 50);
            const { el, setLines } = renderLog(makeLines(3));
            el.scrollTop = 0;
            fireEvent.scroll(el); // unstick
            setLines([]); // Retry clears the log
            scroll.mockReturnValue(400);
            setLines(makeLines(1));
            await vi.waitFor(() => expect(el.scrollTop).toBe(400));
        });
    });
});
