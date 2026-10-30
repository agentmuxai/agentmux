// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The downloads strip: Cancel and Show in folder call the model with the
 * host's id and path, and nothing shows when there are no downloads
 * (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BrowserViewModel } from "./browser-model";
import { DownloadStrip } from "./download-strip";
import { applyDownloadUpdate, type DownloadRow, type DownloadUpdate } from "./downloads";

afterEach(() => cleanup());

const upd = (over: Partial<DownloadUpdate>): DownloadUpdate => ({
    block_id: "blk",
    download_id: 1,
    name: "report.pdf",
    state: "running",
    received: 4_404_019,
    total: 18_874_368,
    ...over,
});

function setup(updates: DownloadUpdate[]) {
    const rows = updates.reduce<DownloadRow[]>((acc, u) => applyDownloadUpdate(acc, u), []);
    const [downloads] = createSignal(rows);
    const model = {
        downloadsAtom: downloads,
        cancelDownload: vi.fn(),
        revealDownload: vi.fn(),
        dismissDownload: vi.fn(),
    };
    render(() => <DownloadStrip model={model as unknown as BrowserViewModel} />);
    return model;
}

describe("DownloadStrip", () => {
    it("draws nothing without downloads", () => {
        setup([]);
        expect(screen.queryByRole("status", { name: "Downloads" })).toBeNull();
    });

    it("shows a running download's progress and Cancel calls the model with the host's id", () => {
        const model = setup([upd({ download_id: 7 })]);
        expect(screen.getByText("report.pdf")).toBeTruthy();
        expect(screen.getByText("4.2 MB of 18.0 MB")).toBeTruthy();
        fireEvent.click(screen.getByText("Cancel"));
        expect(model.cancelDownload).toHaveBeenCalledWith(7);
        expect(screen.queryByText("Show in folder")).toBeNull();
        expect(screen.queryByText("Hide")).toBeNull();
    });

    it("shows an unknown total as received bytes only", () => {
        setup([upd({ total: -1 })]);
        expect(screen.getByText("4.2 MB")).toBeTruthy();
    });

    it("offers Show in folder with the host's path once done, and no Cancel", () => {
        const model = setup([upd({ state: "done", path: "C:/Users/me/Downloads/report.pdf" })]);
        expect(screen.getByText("saved")).toBeTruthy();
        fireEvent.click(screen.getByText("Show in folder"));
        expect(model.revealDownload).toHaveBeenCalledWith("C:/Users/me/Downloads/report.pdf");
        expect(screen.queryByText("Cancel")).toBeNull();
        fireEvent.click(screen.getByText("Hide"));
        expect(model.dismissDownload).toHaveBeenCalledWith("1");
    });

    it("says a failure in words and offers nothing to retry", () => {
        setup([upd({ state: "failed", reason: "The download failed: file no space." })]);
        expect(screen.getByText("The download failed: file no space.")).toBeTruthy();
        expect(screen.queryByText("Cancel")).toBeNull();
        expect(screen.queryByText("Show in folder")).toBeNull();
    });

    it("shows a cancelled download as cancelled", () => {
        setup([upd({ state: "cancelled" })]);
        expect(screen.getByText("cancelled")).toBeTruthy();
    });

    it("shows the flood line", () => {
        setup([upd({ download_id: null, state: "ignored", reason: "This page tried to start more downloads. Ignored." })]);
        expect(screen.getByText("This page tried to start more downloads. Ignored.")).toBeTruthy();
    });
});
