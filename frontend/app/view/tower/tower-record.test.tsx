// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://127.0.0.1:1" }));
vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({}) }));

import { LongRun } from "./tower-record";

const MB = 1024 * 1024;
const H = 3_600_000;
const NOW = 1_000 * H;

const hours = Array.from({ length: 30 }, (_, i) => ({
    h: NOW - (30 - i) * H,
    n: 12,
    v: "0.60.33",
    up_s: 100_000 + i * 3_600,
    restarts: 0,
    last_t: NOW - (29 - i) * H - 60_000,
    am: [1, (1_000 + i * 10) * MB, 1_300 * MB, 1_400 * MB],
    parts: [["core", [1, 400 * MB, 450 * MB, 500 * MB]]],
    pages: [{ w: "main", heap: [1, 310, 330, 350], dom: [1, 18_000, 19_000, 20_000], rows: [0, 4_100, 4_200, 4_300] }],
}));

let urls: string[] = [];
function answer(rolls: unknown[], stalls: unknown[], matched = stalls.length) {
    vi.stubGlobal(
        "fetch",
        vi.fn(async (url: string) => {
            urls.push(url);
            const body = url.includes("kind=events") ? { matched, lines: stalls } : { matched: rolls.length, lines: rolls };
            return { ok: true, status: 200, json: async () => body };
        })
    );
}

describe("the Memory view's over-days panel", () => {
    beforeEach(() => {
        urls = [];
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("reads the rollups and the stalls and shows the chart, the restarts, the stalls and each window's page", async () => {
        answer(hours, [{ t: NOW - H, kind: "ui_stall", ms: 55_000 }]);
        render(() => <LongRun now={() => NOW} />);
        await waitFor(() => expect(screen.getByTestId("tower-memory-long-facts")).toBeTruthy());
        expect(urls.some((u) => u.includes("kind=rollup") && u.includes("limit=170"))).toBe(true);
        expect(urls.some((u) => u.includes("kind=events") && u.includes("limit=500"))).toBe(true);
        expect(screen.getByTestId("tower-memory-long-restarts").textContent).toContain("srv restarted 0 times");
        expect(screen.getByTestId("tower-memory-long-stalls").textContent).toContain("1 UI stall (longest 55 s)");
        expect(screen.getByTestId("tower-memory-long-stalls").textContent).not.toContain("newest");
        const page = screen.getByTestId("tower-memory-long-page").textContent ?? "";
        expect(page).toContain("Window main");
        expect(page).toContain("JS heap 310 MB");
        expect(page).toContain("4,100 rows");
        expect(screen.queryByTestId("tower-memory-long-empty")).toBeNull();
    });

    it("says the newest stalls are shown when the window held more than the answer", async () => {
        answer(hours, [{ t: 1, kind: "srv_stall", ms: 4_000 }], 900);
        render(() => <LongRun now={() => NOW} />);
        await waitFor(() => expect(screen.getByTestId("tower-memory-long-stalls")).toBeTruthy());
        expect(screen.getByTestId("tower-memory-long-stalls").textContent).toContain("newest");
    });

    it("says it fills in over time when the record holds fewer than two hours", async () => {
        answer(hours.slice(0, 1), []);
        render(() => <LongRun now={() => NOW} />);
        await waitFor(() => expect(screen.getByTestId("tower-memory-long-empty").textContent).toContain("fills in"));
    });

    it("says the record could not be read when the server refuses", async () => {
        vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 500, json: async () => ({}) })));
        render(() => <LongRun now={() => NOW} />);
        await waitFor(() => expect(screen.getByTestId("tower-memory-long-empty").textContent).toContain("could not be read"));
    });

    it("asks again for the other span when it is chosen", async () => {
        answer(hours, []);
        render(() => <LongRun now={() => NOW} />);
        await waitFor(() => expect(screen.getByTestId("tower-memory-long-facts")).toBeTruthy());
        urls = [];
        fireEvent.click(screen.getByText("30d"));
        await waitFor(() => expect(urls.some((u) => u.includes("kind=rollup") && u.includes("limit=722"))).toBe(true));
    });
});
