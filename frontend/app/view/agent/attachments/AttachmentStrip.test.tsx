// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The strip of a sent message's attachments lays itself out from the width its list reports. A width of 0 (the pane
 * is hidden) must not collapse it, and a width change that fits the same tiles must not touch them.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import { AttachmentStrip } from "./AttachmentStrip";

let report: (width: number) => void = () => {};

beforeEach(() => {
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
