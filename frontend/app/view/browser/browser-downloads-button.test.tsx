// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Downloads button: it appears once the pane has a finished download, and its menu lines show a saved file
 * in its folder with the host's path (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5).
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DownloadsButton, pastEntry } from "./browser-downloads-button";
import type { BrowserViewModel } from "./browser-model";
import type { DownloadRow } from "./downloads";

afterEach(() => cleanup());

const row = (over: Partial<DownloadRow>): DownloadRow => ({
    key: "past-1",
    id: 1,
    name: "report.pdf",
    state: "done",
    received: 18_000_000,
    total: 18_000_000,
    path: "C:/Users/me/Downloads/report.pdf",
    ...over,
});

describe("pastEntry", () => {
    it("shows a saved file in its folder with the host's path", () => {
        const reveal = vi.fn();
        const entry = pastEntry(row({}), reveal);
        expect(entry).toMatchObject({ label: "report.pdf", sublabel: "17.2 MB", disabled: false });
        entry.onSelect?.();
        expect(reveal).toHaveBeenCalledWith("C:/Users/me/Downloads/report.pdf");
    });

    it("lists a cancelled or failed one without a click to make", () => {
        const reveal = vi.fn();
        for (const over of [{ state: "cancelled" as const, path: undefined }, { state: "failed" as const, path: undefined, reason: "The disk is full." }]) {
            const entry = pastEntry(row(over), reveal);
            expect(entry.disabled).toBe(true);
            expect(entry.onSelect).toBeUndefined();
        }
        expect(pastEntry(row({ state: "failed", path: undefined, reason: "The disk is full." }), reveal).sublabel).toBe("The disk is full.");
    });
});

describe("DownloadsButton", () => {
    const setup = (rows: DownloadRow[]) => {
        const [past] = createSignal(rows);
        const model = { pastDownloadsAtom: past, revealDownload: vi.fn(), clearPastDownloads: vi.fn() };
        render(() => <DownloadsButton model={model as unknown as BrowserViewModel} pane={() => undefined} />);
        return model;
    };

    it("is not there before the pane has finished a download", () => {
        setup([]);
        expect(screen.queryByTestId("browser-downloads-button")).toBeNull();
    });

    it("is there once one has", () => {
        setup([row({})]);
        expect(screen.getByTestId("browser-downloads-button").getAttribute("aria-label")).toMatch(/^Downloads/);
    });
});
