// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model, end to end: a live sample that arrives after a stall makes it
 * fetch the samples it missed from history and merge them in, without clearing
 * the series (docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.1).
 */
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const history = vi.hoisted(() => ({ read: vi.fn(async (..._args: unknown[]): Promise<any[]> => []) }));

vi.mock("@/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ settings: {} }) },
    getConnStatusAtom: () => () => ({ status: "connected" }),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { EventReadHistoryCommand: history.read } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { SysinfoViewModel } from "./sysinfo-model";

const BASE = 1_791_900_000_000;
const event = (ts: number, cpu: number) => ({ event: "sysinfo", scopes: ["local"], data: { ts, values: { cpu }, uptime_secs: 1 } });

function model(): SysinfoViewModel {
    const ctx = { blockId: "s1", meta: () => ({}), setMeta: async () => {}, isFocused: () => false, visibility: () => "active" };
    return new SysinfoViewModel(ctx as unknown as PaneTabHostContext, "sysinfo");
}

const real = (m: SysinfoViewModel) => m.dataAtom().filter((d) => d.blank !== 1);

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(BASE + 100_000);
    history.read.mockReset();
    history.read.mockResolvedValue([]);
});

afterEach(() => {
    vi.useRealTimers();
});

describe("SysinfoViewModel gap backfill", () => {
    it("fetches the samples a stall dropped and merges them in, keeping the series", async () => {
        await createRoot(async (dispose) => {
            const m = model();
            await vi.advanceTimersByTimeAsync(0);
            m.appendData({ ts: BASE, cpu: 40 });
            m.appendData({ ts: BASE + 1000, cpu: 41 });
            history.read.mockResolvedValue([
                ...Array.from({ length: 25 }, (_, i) => event(BASE + 1000 + i * 1000, 60 + i)),
            ]);
            const before = history.read.mock.calls.length;

            // 25 s of nothing, then one live sample.
            m.appendData({ ts: BASE + 26_000, cpu: 90 });
            expect(m.dataAtom().some((d) => d.blank === 1 && d.ts > BASE + 1000 && d.ts < BASE + 26_000)).toBe(true);
            await vi.advanceTimersByTimeAsync(0);

            expect(history.read.mock.calls.length).toBe(before + 1);
            expect(real(m).map((d) => d.ts)).toEqual(Array.from({ length: 27 }, (_, i) => BASE + i * 1000));
            // The live value wins where both have the sample; the history fills the rest.
            expect(real(m).find((d) => d.ts === BASE + 1000)?.cpu).toBe(41);
            expect(real(m).find((d) => d.ts === BASE + 10_000)?.cpu).toBe(69);
            expect(m.dataAtom().some((d) => d.blank === 1 && d.ts > BASE && d.ts < BASE + 26_000)).toBe(false);
            expect(m.loadingAtom()).toBe(false);
            dispose();
        });
    });

    it("does not fetch for samples that arrive on time", async () => {
        await createRoot(async (dispose) => {
            const m = model();
            await vi.advanceTimersByTimeAsync(0);
            const before = history.read.mock.calls.length;
            for (let i = 0; i < 10; i++) m.appendData({ ts: BASE + i * 1000, cpu: 40 + i });
            await vi.advanceTimersByTimeAsync(0);
            expect(history.read.mock.calls.length).toBe(before);
            dispose();
        });
    });

    it("keeps the break when history has nothing for the gap, and does not loop on it", async () => {
        await createRoot(async (dispose) => {
            const m = model();
            await vi.advanceTimersByTimeAsync(0);
            m.appendData({ ts: BASE, cpu: 40 });
            const before = history.read.mock.calls.length;
            m.appendData({ ts: BASE + 30_000, cpu: 90 });
            await vi.advanceTimersByTimeAsync(60_000);
            expect(history.read.mock.calls.length).toBe(before + 1);
            expect(m.dataAtom().filter((d) => d.blank === 1)).toHaveLength(2);
            dispose();
        });
    });

    it("drops what a fetch returns if the pane switched machines meanwhile", async () => {
        await createRoot(async (dispose) => {
            const [meta, setMeta] = createSignal<Record<string, unknown>>({});
            const ctx = { blockId: "s1", meta, setMeta: async () => {}, isFocused: () => false, visibility: () => "active" };
            const m = new SysinfoViewModel(ctx as unknown as PaneTabHostContext, "sysinfo");
            await vi.advanceTimersByTimeAsync(0);
            m.appendData({ ts: BASE, cpu: 40 });
            let finish!: (v: any[]) => void;
            history.read.mockImplementationOnce(() => new Promise((r) => (finish = r)));
            m.appendData({ ts: BASE + 30_000, cpu: 90 });
            // The pane is pointed at another machine, which reloads its own series.
            setMeta({ connection: "wsl://Ubuntu" });
            expect(m.connection()).toBe("wsl://Ubuntu");
            history.read.mockResolvedValue([]);
            m.resetData([{ ts: BASE + 60_000, cpu: 5 }]);
            // The old machine's samples arrive, inside the new series' window.
            finish([event(BASE + 10_000, 70), event(BASE + 20_000, 71)]);
            await vi.advanceTimersByTimeAsync(0);
            expect(real(m).map((d) => d.ts)).toEqual([BASE + 60_000]);
            dispose();
        });
    });

    it("asks again for a second stall, but not within five seconds of the first", async () => {
        await createRoot(async (dispose) => {
            const m = model();
            await vi.advanceTimersByTimeAsync(0);
            m.appendData({ ts: BASE, cpu: 40 });
            const before = history.read.mock.calls.length;
            m.appendData({ ts: BASE + 30_000, cpu: 90 });
            await vi.advanceTimersByTimeAsync(1000);
            m.appendData({ ts: BASE + 61_000, cpu: 91 });
            await vi.advanceTimersByTimeAsync(1000);
            expect(history.read.mock.calls.length).toBe(before + 1);
            await vi.advanceTimersByTimeAsync(4000);
            expect(history.read.mock.calls.length).toBe(before + 2);
            dispose();
        });
    });
});
