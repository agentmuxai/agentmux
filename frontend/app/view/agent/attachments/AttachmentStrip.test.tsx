// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The strip of a sent message's attachments lays itself out from the width its list reports. A width of 0 (the pane
 * is hidden) must not collapse it, and a width change that fits the same tiles must not touch them.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const info = vi.hoisted(() => ({ ask: vi.fn() }));

vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AttachmentsInfoCommand: (_c: unknown, d: unknown) => info.ask(d) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AttachmentStrip } from "./AttachmentStrip";

let report: (width: number) => void = () => {};

beforeEach(() => {
    info.ask.mockReset();
    info.ask.mockResolvedValue({ items: [] });
    class FakeObserver {
        constructor(cb: (entries: { contentRect: { width: number } }[]) => void) {
            report = (width) => cb([{ contentRect: { width } }]);
        }
        observe() {}
        disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", FakeObserver);
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

const files = Array.from({ length: 10 }, (_, i) => ({ id: "a".repeat(63) + i, name: `shot-${i}.png`, image: true }));
const tiles = (el: HTMLElement) => [...el.querySelectorAll("li.agent-attachment-tile:not(.agent-attachment-tile--more)")];
const more = (el: HTMLElement) => el.querySelector(".agent-attachment-tile--more .more")?.textContent;

describe("AttachmentStrip layout", () => {
    it("shows as many tiles as the width fits and a +N for the rest", () => {
        const { container } = render(() => <AttachmentStrip attachments={files} />);
        report(600);
        expect(tiles(container)).toHaveLength(7);
        expect(more(container)).toBe("+3");
        report(300);
        expect(tiles(container)).toHaveLength(3);
        expect(more(container)).toBe("+7");
    });

    it("keeps its tiles when the pane reports no width, and does not remount them for a width that fits the same", () => {
        const { container } = render(() => <AttachmentStrip attachments={files} />);
        report(600);
        const before = tiles(container);
        report(0);
        expect(tiles(container)).toEqual(before);
        expect(tiles(container).every((el, i) => el === before[i])).toBe(true);
        report(560);
        expect(tiles(container).every((el, i) => el === before[i])).toBe(true);
        expect(more(container)).toBe("+3");
    });
});

describe("AttachmentStrip details", () => {
    const pdf = { id: "b".repeat(64), name: "report.pdf" };

    it("asks for the details of the shown ids once, and gives a PDF's tile its page count", async () => {
        info.ask.mockResolvedValue({ items: [{ id: pdf.id, kind: "pdf", page_count: 3 }] });
        const { container } = render(() => <AttachmentStrip attachments={[pdf]} />);
        report(600);
        const tile = container.querySelector("li.agent-attachment-tile");
        await vi.waitFor(() => expect(container.querySelector(".agent-attachment-tile__pages")?.textContent).toContain("3"));
        expect(info.ask).toHaveBeenCalledTimes(1);
        expect(info.ask).toHaveBeenCalledWith({ ids: [pdf.id] });
        // The tile was not mounted again when the details arrived.
        expect(container.querySelector("li.agent-attachment-tile")).toBe(tile);
    });

    it("leaves the tile as it was when the details can't be had", async () => {
        info.ask.mockRejectedValue(new Error("srv down"));
        const { container } = render(() => <AttachmentStrip attachments={[pdf]} />);
        report(600);
        await Promise.resolve();
        expect(container.querySelector("li.agent-attachment-tile")).not.toBeNull();
        expect(container.querySelector(".agent-attachment-tile__pages")).toBeNull();
    });

    it("does not ask for an attachment that is already gone", () => {
        render(() => <AttachmentStrip attachments={[{ name: "gone.pdf" }]} />);
        expect(info.ask).not.toHaveBeenCalled();
    });
});
