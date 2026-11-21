// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The status bar's low-disk panel on a host that pushes no health signals (a browser tab): it opens on the Disk readout
 * once for each crossing of the desktop's lines, and a poll at the same level opens nothing. The panel is a stand-in
 * here (LowDiskNotice.test.tsx has the real one); the system-info event is fed by hand.
 */

import { makeTestHostApi } from "@/app/host/test-host";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ emit: null as any }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, cb: (data: any) => void) => {
        h.emit = cb;
        return () => {};
    },
}));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { SysInfo: "sysinfo" } }));
vi.mock("./LowDiskNotice", () => ({
    LowDiskNotice: (p: { notice: { level: string; label?: string; reading: { freeGb: number } }; onDismiss: () => void }) => (
        <div
            data-stand="LowDiskNotice"
            data-level={p.notice.level}
            data-label={p.notice.label}
            data-free={p.notice.reading.freeGb}
            onClick={p.onDismiss}
        />
    ),
}));

import { SystemStats } from "./SystemStats";

const realApi = window.api;
let host: HTMLElement;

/** A system-info event for a 100 GB volume mounted at "/" with `freeGb` free. */
const send = (freeGb: number) =>
    h.emit({
        values: {
            "disk:watch:free_gb": freeGb,
            "disk:watch:total_gb": 100,
            "disk:watch:free_pct": freeGb,
            "disk:vol:/:free_gb": freeGb,
            "disk:vol:/:total_gb": 100,
            "disk:vol:/:watch": 1,
        },
    });
const panel = () => host.querySelector<HTMLElement>("[data-stand='LowDiskNotice']");

function mount() {
    host = render(() => <SystemStats />, { container: document.body.appendChild(document.createElement("div")) }).container;
}

beforeEach(() => {
    h.emit = null;
    window.api = makeTestHostApi();
});
afterEach(() => {
    cleanup();
    window.api = realApi;
});

describe("the low-disk panel on the Disk readout", () => {
    it("stays closed while there is room", () => {
        mount();
        send(60);
        send(40);
        expect(panel()).toBeNull();
    });

    it("opens once when free space falls under the warn line, and not at the polls that follow", () => {
        mount();
        send(60);
        send(9); // 9 GB of 100 GB: under the 10 GB line
        expect(panel()?.dataset.level).toBe("warn");
        fireEvent.click(panel()!); // dismissed
        expect(panel()).toBeNull();
        // The same level, and numbers wobbling inside it, open nothing.
        send(9);
        send(8.5);
        send(10.5); // over the line, inside the margin
        expect(panel()).toBeNull();
    });

    it("says which volume and how much while it is open, and keeps the numbers current", () => {
        mount();
        send(9);
        expect(panel()?.dataset.label).toBe("/");
        expect(panel()?.dataset.free).toBe("9");
        send(8);
        expect(panel()?.dataset.free).toBe("8");
    });

    it("opens again for critical, after a warning was dismissed", () => {
        mount();
        send(9);
        fireEvent.click(panel()!);
        send(2.5); // under the 3 GB line
        expect(panel()?.dataset.level).toBe("critical");
    });

    it("closes by itself when the volume recovers, and opens again at the next drop", () => {
        mount();
        send(9);
        expect(panel()).not.toBeNull();
        send(30); // clear of the line and its margin
        expect(panel()).toBeNull();
        send(9);
        expect(panel()?.dataset.level).toBe("warn");
    });

    it("does nothing on a host that pushes the signal itself", () => {
        window.api = makeTestHostApi({}, { hostHealthSignals: true });
        mount();
        send(9);
        send(2);
        expect(panel()).toBeNull();
    });

    it("ignores a reading with no total, rather than guessing", () => {
        mount();
        h.emit({ values: { "disk:watch:free_gb": 1, "disk:watch:free_pct": 1 } });
        expect(panel()).toBeNull();
    });
});
