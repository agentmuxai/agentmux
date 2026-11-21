// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When the audio device is opened (`new AudioContext()`, which blocks the page for as long as the machine's audio
 * stack takes): in idle time after the app's first frame, never from the first gesture's handler or any timer of the
 * service, and never with sounds off. The real player and a fake `AudioContext` that counts its constructions, so the
 * test sees exactly when the blocking call is made. The idle wait itself is open-when-idle.test.ts.
 */

import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The mock factories below only touch `h` and `idle` when the service calls them, after this has run.
const idle = vi.hoisted(() => ({ runs: [] as Array<() => void>, cancels: 0 }));
const h = (() => {
    const settings = new Map<string, [() => unknown, (v: unknown) => void]>();
    const setting = (key: string) => {
        let s = settings.get(key);
        if (!s) {
            const [get, set] = createSignal<unknown>(undefined);
            s = [get, (v: unknown) => set(() => v)];
            settings.set(key, s);
        }
        return s;
    };
    return { setting, reset: () => settings.forEach((s) => s[1](undefined)) };
})();

vi.mock("../open-when-idle", () => ({
    whenIdleAfterFirstPaint: (run: () => void) => {
        idle.runs.push(run);
        return () => {
            idle.cancels++;
        };
    },
}));
vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => h.setting(key)[0](),
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { blockFocusAtom: () => () => null },
}));
vi.mock("@/app/window/window-focus", () => ({
    makeWindowFocusSignal: () => () => true,
}));
vi.mock("@/app/store/agent-pane-state-store", () => ({
    addEventListener: () => () => undefined,
}));

import { SETTLE_MS } from "../after-gesture";
import { __getSoundPlayer, __getToolTonesPlayer, __resetSoundService, installSoundService } from "../sound-service";

class FakeAudioContext {
    static made = 0;
    state = "suspended";
    currentTime = 0;
    destination = {};
    resume = vi.fn(async () => {
        this.state = "running";
    });
    constructor() {
        FakeAudioContext.made++;
    }
    createGain() {
        return { gain: { value: 1 }, connect: vi.fn() };
    }
}

let addSpy: ReturnType<typeof vi.spyOn>;
let warn: ReturnType<typeof vi.spyOn>;
let cleanup: (() => void) | null = null;

const perfLines = () => warn.mock.calls.map((c) => String(c[0])).filter((l) => l.startsWith("[perf]"));
const setSounds = (v: boolean | undefined) => h.setting("notify:sounds:enabled")[1](v);

function install(): void {
    cleanup = installSoundService();
}

/** Run every idle request the service has made so far, as the page's idle time would. */
function idleTimeComes(): void {
    const runs = idle.runs.splice(0);
    for (const run of runs) run();
}

/** A real press that ends, then the settle time: what the first gesture's handler waits for before it does anything slow. */
async function firstGesture(): Promise<void> {
    const call = addSpy.mock.calls.find((c) => c[0] === "pointerdown");
    if (!call) throw new Error("no pointerdown listener was added");
    const done = (call[1] as (e: Event) => Promise<void>)({ isTrusted: true, type: "pointerdown" } as unknown as Event);
    document.dispatchEvent(new Event("pointerup"));
    await vi.advanceTimersByTimeAsync(SETTLE_MS);
    await done;
}

beforeEach(() => {
    vi.useFakeTimers();
    h.reset();
    idle.runs.length = 0;
    idle.cancels = 0;
    FakeAudioContext.made = 0;
    __resetSoundService();
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("no network in a test")));
    addSpy = vi.spyOn(document, "addEventListener");
    warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const toolTones = __getToolTonesPlayer();
    vi.spyOn(toolTones, "attach").mockImplementation(() => undefined);
    vi.spyOn(toolTones, "isAttached").mockReturnValue(false);
});

afterEach(() => {
    delete (navigator as unknown as { userActivation?: unknown }).userActivation;
    cleanup?.();
    cleanup = null;
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("the audio device is opened in idle time", () => {
    it("not at install: the open is asked for when the page is idle, and the context then stays suspended", async () => {
        install();
        expect(FakeAudioContext.made).toBe(0);
        expect(idle.runs).toHaveLength(1);

        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);

        expect(FakeAudioContext.made).toBe(1);
        expect(__getSoundPlayer().isPrimed()).toBe(true);
        // No gesture yet, so nothing asks the browser to start it.
        expect((__getSoundPlayer().getAudioContext() as unknown as FakeAudioContext).resume).not.toHaveBeenCalled();
    });

    it("writes one [perf] line with what the open cost and why it was made", async () => {
        install();
        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);
        expect(perfLines()).toHaveLength(1);
        expect(perfLines()[0]).toMatch(/^\[perf\] audio-open \d+ms why=idle$/);
    });

    it("the first gesture then only lets the open context run: no second open, no second line", async () => {
        install();
        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);

        await firstGesture();

        expect(FakeAudioContext.made).toBe(1);
        expect((__getSoundPlayer().getAudioContext() as unknown as FakeAudioContext).resume).toHaveBeenCalledTimes(1);
        expect(perfLines()).toHaveLength(1);
    });

    it("a gesture that comes before the idle time opens it after the gesture, as before, and the idle time then adds nothing", async () => {
        install();
        await firstGesture();
        expect(FakeAudioContext.made).toBe(1);
        expect((__getSoundPlayer().getAudioContext() as unknown as FakeAudioContext).resume).toHaveBeenCalledTimes(1);
        expect(perfLines()).toHaveLength(1);
        expect(perfLines()[0]).toMatch(/why=gesture$/);

        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);
        expect(FakeAudioContext.made).toBe(1);
        expect(perfLines()).toHaveLength(1);
    });

    it("the cleanup withdraws an open that was still waiting", () => {
        install();
        expect(idle.cancels).toBe(0);
        cleanup!();
        cleanup = null;
        expect(idle.cancels).toBe(1);
    });
});

describe("the audio device is never opened with sounds off", () => {
    it("not in idle time and not from the first gesture", async () => {
        setSounds(false);
        install();
        expect(idle.runs).toHaveLength(0);
        idleTimeComes();
        await firstGesture();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(FakeAudioContext.made).toBe(0);
        expect(perfLines()).toEqual([]);
    });

    it("turned off after the open was asked for and before the idle time comes", async () => {
        install();
        expect(idle.runs).toHaveLength(1);
        setSounds(false);
        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);
        expect(FakeAudioContext.made).toBe(0);
    });

    it("turned on later, it is opened once in idle time, and runs at once when the page has had a gesture", async () => {
        Object.defineProperty(navigator, "userActivation", { value: { hasBeenActive: true }, configurable: true });
        setSounds(false);
        install();
        expect(idle.runs).toHaveLength(0);

        setSounds(true);
        expect(idle.runs).toHaveLength(1);
        // Off and on again before the idle time comes is still one request.
        setSounds(false);
        setSounds(true);
        expect(idle.runs).toHaveLength(1);

        idleTimeComes();
        await vi.advanceTimersByTimeAsync(0);
        expect(FakeAudioContext.made).toBe(1);
        expect((__getSoundPlayer().getAudioContext() as unknown as FakeAudioContext).resume).toHaveBeenCalledTimes(1);
    });
});
