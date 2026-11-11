// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What `installSoundService()` wires up, one piece at a time: the install
 * guard and its cleanup, priming on the first real gesture, the reactive
 * volume / disable / focus effects over the waiting tones, and the bus
 * listener's coalesce and flash details. Unlike the policy tests next to
 * it, the settings and focus here are real signals, so the effects run.
 */

import { batch, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// ─── Module mocks ─────────────────────────────────────────────────────

type Sig = [() => unknown, (v: unknown) => void];

// The mock factories below only touch `h` when the service calls them, after this has run.
const h = (() => {
    const make = (init: unknown): Sig => {
        const [get, set] = createSignal<unknown>(init);
        return [get, (v: unknown) => set(() => v)];
    };
    const settings = new Map<string, Sig>();
    const setting = (key: string): Sig => {
        let s = settings.get(key);
        if (!s) {
            s = make(undefined);
            settings.set(key, s);
        }
        return s;
    };
    const focus = make(null);
    const win = make(true);
    return {
        setting,
        focus,
        win,
        reset: () => {
            for (const s of settings.values()) s[1](undefined);
            focus[1](null);
            win[1](true);
        },
    };
})();

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => h.setting(key)[0](),
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: {
        get blockFocusAtom() {
            return () => h.focus[0]() as string | null;
        },
    },
}));
vi.mock("@/app/window/window-focus", () => ({
    makeWindowFocusSignal: () => () => h.win[0]() as boolean,
}));

let captured: ((blockId: string, event: { type: string; [k: string]: unknown }) => void) | null = null;
const paneAdd = vi.fn();
vi.mock("@/app/store/agent-pane-state-store", () => ({
    addEventListener: (l: typeof captured) => {
        paneAdd();
        captured = l;
        return () => {
            captured = null;
        };
    },
}));

// ─── Import after mocks ───────────────────────────────────────────────

import { __resetActivityFlash, onActivityFlash, type FlashTarget } from "../../activity-flash";
import { audibleFlashDelayMs } from "../flash-patterns";
import { DEFAULT_MASTER_VOLUME, DEFAULT_TOOLTONES_VOLUME, DEFAULT_WAITING_VOLUME } from "../sound-defaults";
import { __resetSoundListeners, notify } from "../sound-events";
import {
    __getSoundPlayer,
    __getToolTonesPlayer,
    __getWaitingTones,
    __resetSoundService,
    installSoundService,
} from "../sound-service";
import { WAITING_LOOP_MS, WaitingTonePlayer } from "../waiting-tone-player";

// ─── Helpers ──────────────────────────────────────────────────────────

const player = __getSoundPlayer();
const toolTones = __getToolTonesPlayer();

const flashes: FlashTarget[] = [];
let offFlash: () => void;
let cleanup: (() => void) | null = null;
let addSpy: ReturnType<typeof vi.spyOn>;
let removeSpy: ReturnType<typeof vi.spyOn>;
let playSpy: ReturnType<typeof vi.spyOn>;
let masterSpy: ReturnType<typeof vi.spyOn>;
let toolVolSpy: ReturnType<typeof vi.spyOn>;
let wpStart: ReturnType<typeof vi.spyOn>;
let wpStop: ReturnType<typeof vi.spyOn>;
let wpSetVol: ReturnType<typeof vi.spyOn>;
let wpAttach: ReturnType<typeof vi.spyOn>;
let wpIsAttached: ReturnType<typeof vi.spyOn>;

function set(key: string, value: unknown): void {
    h.setting(key)[1](value);
}

function install(): () => void {
    cleanup = installSoundService();
    return cleanup;
}

function fire(blockId: string, event: { type: string; [k: string]: unknown }): void {
    if (!captured) throw new Error("multicast listener was never installed");
    captured(blockId, event);
}

function startWaiting(blockId: string): void {
    fire(blockId, { type: "waiting-for-input" });
}

function flashesFor(blockId: string): number {
    return flashes.filter((f) => f.blockId === blockId).length;
}

type Listener = (e: Event) => Promise<void>;

function primeListener(type: "pointerdown" | "keydown"): { fn: Listener; options: unknown } {
    const call = addSpy.mock.calls.find((c) => c[0] === type);
    if (!call) throw new Error(`no ${type} listener was added`);
    return { fn: call[1] as Listener, options: call[2] };
}

function trusted(): Event {
    return { isTrusted: true } as unknown as Event;
}

const fakeCtx = { currentTime: 1, baseLatency: 0, outputLatency: 0 } as unknown as AudioContext;
const fakeMaster = {} as unknown as GainNode;

beforeEach(() => {
    vi.useFakeTimers();
    h.reset();
    paneAdd.mockClear();
    __resetSoundService();
    __resetSoundListeners();
    __resetActivityFlash();
    captured = null;
    flashes.length = 0;
    offFlash = onActivityFlash((t) => flashes.push(t));
    addSpy = vi.spyOn(document, "addEventListener");
    removeSpy = vi.spyOn(document, "removeEventListener");
    playSpy = vi.spyOn(player, "play").mockImplementation(() => null);
    masterSpy = vi.spyOn(player, "setMasterGain");
    toolVolSpy = vi.spyOn(toolTones, "setVolume");
    wpStart = vi.spyOn(WaitingTonePlayer.prototype, "start").mockImplementation(() => {});
    wpStop = vi.spyOn(WaitingTonePlayer.prototype, "stop").mockImplementation(() => Promise.resolve());
    wpSetVol = vi.spyOn(WaitingTonePlayer.prototype, "setVolume").mockImplementation(() => {});
    wpAttach = vi.spyOn(WaitingTonePlayer.prototype, "attach").mockImplementation(() => {});
    wpIsAttached = vi.spyOn(WaitingTonePlayer.prototype, "isAttached").mockReturnValue(false);
});

afterEach(() => {
    cleanup?.();
    cleanup = null;
    offFlash();
    vi.restoreAllMocks();
    vi.useRealTimers();
});

// ─── Install guard and cleanup ────────────────────────────────────────

describe("installSoundService: install and cleanup", () => {
    it("a second install is a no-op, and its cleanup does not uninstall the first", () => {
        install();
        const second = installSoundService();
        second();
        const third = installSoundService();
        third();
        expect(paneAdd).toHaveBeenCalledTimes(1);
        expect(addSpy.mock.calls.filter((c) => c[0] === "pointerdown")).toHaveLength(1);
        expect(addSpy.mock.calls.filter((c) => c[0] === "keydown")).toHaveLength(1);
    });

    it("cleanup removes every listener, stops the volume effects, and allows a fresh install", () => {
        let now = 1000;
        vi.spyOn(performance, "now").mockImplementation(() => now);
        const stop = install();
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        const before = playSpy.mock.calls.length;
        expect(before).toBe(1);
        now = 5000;
        const down = primeListener("pointerdown").fn;
        const key = primeListener("keydown").fn;

        stop();
        cleanup = null;

        expect(removeSpy).toHaveBeenCalledWith("pointerdown", down, true);
        expect(removeSpy).toHaveBeenCalledWith("keydown", key, true);
        expect(captured).toBeNull();
        notify("agent.turn.error", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(before);
        const calls = masterSpy.mock.calls.length;
        set("notify:sounds:volume", 0.9);
        expect(masterSpy).toHaveBeenCalledTimes(calls);

        install();
        expect(paneAdd).toHaveBeenCalledTimes(2);
        expect(captured).not.toBeNull();
    });
});

// ─── Priming ──────────────────────────────────────────────────────────

describe("installSoundService: priming on the first real gesture", () => {
    it("arms pointerdown and keydown in the capture phase", () => {
        install();
        expect(primeListener("pointerdown").options).toEqual({ capture: true });
        expect(primeListener("keydown").options).toEqual({ capture: true });
    });

    it("ignores an untrusted event: nothing is primed and both listeners stay armed", () => {
        const prime = vi.spyOn(player, "prime").mockResolvedValue();
        install();
        void primeListener("pointerdown").fn({ isTrusted: false } as unknown as Event);
        void primeListener("keydown").fn({ isTrusted: false } as unknown as Event);
        expect(prime).not.toHaveBeenCalled();
        expect(removeSpy.mock.calls.filter((c) => c[0] === "pointerdown" || c[0] === "keydown")).toHaveLength(0);
    });

    it("a trusted pointerdown primes once and takes both listeners down", async () => {
        const prime = vi.spyOn(player, "prime").mockResolvedValue();
        install();
        const down = primeListener("pointerdown").fn;
        const key = primeListener("keydown").fn;
        await down(trusted());
        expect(prime).toHaveBeenCalledTimes(1);
        expect(removeSpy).toHaveBeenCalledWith("pointerdown", down, true);
        expect(removeSpy).toHaveBeenCalledWith("keydown", key, true);
    });

    it("a trusted keydown does the same", async () => {
        const prime = vi.spyOn(player, "prime").mockResolvedValue();
        install();
        const down = primeListener("pointerdown").fn;
        const key = primeListener("keydown").fn;
        await key(trusted());
        expect(prime).toHaveBeenCalledTimes(1);
        expect(removeSpy).toHaveBeenCalledWith("pointerdown", down, true);
        expect(removeSpy).toHaveBeenCalledWith("keydown", key, true);
    });

    it("attaches the tool tones and the waiting players only once priming has finished", async () => {
        let finish!: () => void;
        vi.spyOn(player, "prime").mockImplementation(
            () =>
                new Promise<void>((resolve) => {
                    finish = resolve;
                })
        );
        const ctx = vi.spyOn(player, "getAudioContext").mockReturnValue(null);
        const master = vi.spyOn(player, "getMasterGain").mockReturnValue(null);
        const attach = vi.spyOn(toolTones, "attach").mockImplementation(() => {});
        vi.spyOn(toolTones, "isAttached").mockReturnValue(false);
        install();
        startWaiting("blk-a");
        startWaiting("blk-b");
        expect(wpAttach).not.toHaveBeenCalled();

        const pending = primeListener("pointerdown").fn(trusted());
        await Promise.resolve();
        expect(attach).not.toHaveBeenCalled();
        expect(wpAttach).not.toHaveBeenCalled();

        ctx.mockReturnValue(fakeCtx);
        master.mockReturnValue(fakeMaster);
        finish();
        await pending;
        expect(attach).toHaveBeenCalledTimes(1);
        expect(attach).toHaveBeenCalledWith(fakeCtx, fakeMaster);
        expect(wpAttach).toHaveBeenCalledTimes(2);
        expect(wpAttach).toHaveBeenCalledWith(fakeCtx, fakeMaster);
    });

    it("leaves already-attached players alone", async () => {
        vi.spyOn(player, "prime").mockResolvedValue();
        vi.spyOn(player, "getAudioContext").mockReturnValue(null);
        vi.spyOn(player, "getMasterGain").mockReturnValue(null);
        const attach = vi.spyOn(toolTones, "attach").mockImplementation(() => {});
        vi.spyOn(toolTones, "isAttached").mockReturnValue(true);
        install();
        startWaiting("blk-a");
        wpIsAttached.mockReturnValue(true);
        vi.spyOn(player, "getAudioContext").mockReturnValue(fakeCtx);
        vi.spyOn(player, "getMasterGain").mockReturnValue(fakeMaster);
        await primeListener("keydown").fn(trusted());
        expect(attach).not.toHaveBeenCalled();
        expect(wpAttach).not.toHaveBeenCalled();
    });

    it("attaches nothing when the context is missing, or when the master gain is", async () => {
        vi.spyOn(player, "prime").mockResolvedValue();
        const attach = vi.spyOn(toolTones, "attach").mockImplementation(() => {});
        vi.spyOn(toolTones, "isAttached").mockReturnValue(false);
        const ctx = vi.spyOn(player, "getAudioContext").mockReturnValue(null);
        const master = vi.spyOn(player, "getMasterGain").mockReturnValue(fakeMaster);
        install();
        startWaiting("blk-a");
        await primeListener("pointerdown").fn(trusted());
        expect(attach).not.toHaveBeenCalled();
        expect(wpAttach).not.toHaveBeenCalled();

        ctx.mockReturnValue(fakeCtx);
        master.mockReturnValue(null);
        await primeListener("keydown").fn(trusted());
        expect(attach).not.toHaveBeenCalled();
        expect(wpAttach).not.toHaveBeenCalled();
    });
});

// ─── Volume settings ──────────────────────────────────────────────────

describe("installSoundService: volume settings reach their gain nodes", () => {
    it("starts from the defaults when nothing is set", () => {
        install();
        expect(masterSpy).toHaveBeenLastCalledWith(DEFAULT_MASTER_VOLUME);
        expect(toolVolSpy).toHaveBeenLastCalledWith(DEFAULT_TOOLTONES_VOLUME);
    });

    it("the master volume follows notify:sounds:volume, and a non-number falls back to the default", () => {
        install();
        set("notify:sounds:volume", 0.9);
        expect(masterSpy).toHaveBeenLastCalledWith(0.9);
        set("notify:sounds:volume", 0);
        expect(masterSpy).toHaveBeenLastCalledWith(0);
        set("notify:sounds:volume", "loud");
        expect(masterSpy).toHaveBeenLastCalledWith(DEFAULT_MASTER_VOLUME);
    });

    it("the tool-tones volume follows notify:tooltones:volume only", () => {
        install();
        masterSpy.mockClear();
        toolVolSpy.mockClear();
        set("notify:tooltones:volume", 0.8);
        expect(toolVolSpy).toHaveBeenLastCalledWith(0.8);
        expect(masterSpy).not.toHaveBeenCalled();
        set("notify:tooltones:volume", "loud");
        expect(toolVolSpy).toHaveBeenLastCalledWith(DEFAULT_TOOLTONES_VOLUME);
        toolVolSpy.mockClear();
        set("notify:sounds:volume", 0.3);
        expect(toolVolSpy).not.toHaveBeenCalled();
    });

    it("the waiting volume reaches every waiting player, and a non-number falls back to the default", () => {
        install();
        startWaiting("blk-a");
        startWaiting("blk-b");
        wpSetVol.mockClear();
        set("notify:sounds:waiting:volume", 0.7);
        expect(wpSetVol).toHaveBeenCalledTimes(2);
        expect(wpSetVol).toHaveBeenLastCalledWith(0.7);
        set("notify:sounds:waiting:volume", "loud");
        expect(wpSetVol).toHaveBeenLastCalledWith(DEFAULT_WAITING_VOLUME);
        wpSetVol.mockClear();
        masterSpy.mockClear();
        set("notify:sounds:volume", 0.2);
        expect(wpSetVol).not.toHaveBeenCalled();
    });

    it("the waiting volume is not taken from the master volume", () => {
        install();
        startWaiting("blk-a");
        wpSetVol.mockClear();
        set("notify:sounds:volume", 0.2);
        set("notify:tooltones:volume", 0.2);
        expect(wpSetVol).not.toHaveBeenCalled();
    });
});

// ─── Turning the waiting tone off ─────────────────────────────────────

describe("installSoundService: switching the waiting tone off mid-loop", () => {
    it("the master switch stops every waiting tone", () => {
        install();
        startWaiting("blk-a");
        startWaiting("blk-b");
        startWaiting("blk-c");
        wpStop.mockClear();
        set("notify:sounds:enabled", false);
        expect(__getWaitingTones().size).toBe(0);
        expect(wpStop).toHaveBeenCalledTimes(3);
    });

    it("the waiting sound's own switch stops every waiting tone", () => {
        install();
        startWaiting("blk-a");
        startWaiting("blk-b");
        set("notify:sound:agent.waiting.for.input", false);
        expect(__getWaitingTones().size).toBe(0);
    });

    it("leaves the tones alone for anything that is not exactly false", () => {
        install();
        startWaiting("blk-a");
        set("notify:sounds:enabled", true);
        set("notify:sound:agent.waiting.for.input", true);
        set("notify:sounds:enabled", null);
        set("notify:sound:agent.waiting.for.input", 0);
        set("notify:sound:agent.waiting.for.input", undefined);
        expect(__getWaitingTones().size).toBe(1);
    });

    it("another sound's switch does not touch the waiting tones", () => {
        install();
        startWaiting("blk-a");
        set("notify:sound:agent.turn.complete", false);
        set("notify:sound:agent.turn.error", false);
        expect(__getWaitingTones().size).toBe(1);
    });

    it("also silences the tab flash and the auto-stop timer", () => {
        install();
        startWaiting("blk-a");
        expect(flashesFor("blk-a")).toBe(1);
        set("notify:sounds:enabled", false);
        vi.advanceTimersByTime(WAITING_LOOP_MS * 3);
        expect(flashesFor("blk-a")).toBe(1);
        expect(vi.getTimerCount()).toBe(0);
    });
});

// ─── Waiting tones follow focus ───────────────────────────────────────

describe("installSoundService: waiting tones follow focus", () => {
    it("suspends a tone when its pane gains focus, and restarts it from the top when focus leaves", () => {
        install();
        startWaiting("blk-a");
        expect(wpStart).toHaveBeenCalledTimes(1);
        expect(flashesFor("blk-a")).toBe(1);

        h.focus[1]("blk-a");
        expect(wpStop).toHaveBeenCalledTimes(1);
        expect(wpStop.mock.contexts[0]).toBe(__getWaitingTones().get("blk-a"));
        expect(__getWaitingTones().has("blk-a")).toBe(true);
        vi.advanceTimersByTime(WAITING_LOOP_MS * 3);
        expect(flashesFor("blk-a")).toBe(1);

        h.focus[1](null);
        expect(wpStart).toHaveBeenCalledTimes(2);
        expect(wpStart.mock.contexts[1]).toBe(__getWaitingTones().get("blk-a"));
        expect(flashesFor("blk-a")).toBe(2);
        vi.advanceTimersByTime(WAITING_LOOP_MS);
        expect(flashesFor("blk-a")).toBe(3);
    });

    it("restarts at the waiting volume, defaulting when it is not set", () => {
        install();
        set("notify:sounds:volume", 0.9);
        startWaiting("blk-a");
        h.focus[1]("blk-a");
        h.focus[1](null);
        expect(wpStart).toHaveBeenLastCalledWith(DEFAULT_WAITING_VOLUME);

        h.focus[1]("blk-a");
        set("notify:sounds:waiting:volume", 0.4);
        h.focus[1](null);
        expect(wpStart).toHaveBeenLastCalledWith(0.4);
    });

    it("does not suspend a tone for a different pane's focus", () => {
        install();
        startWaiting("blk-a");
        h.focus[1]("blk-b");
        expect(wpStop).not.toHaveBeenCalled();
        expect(wpStart).toHaveBeenCalledTimes(1);
    });

    it("does not suspend while the window is blurred, and suspends when it comes back to the pane", () => {
        install();
        startWaiting("blk-a");
        h.win[1](false);
        h.focus[1]("blk-a");
        expect(wpStop).not.toHaveBeenCalled();
        h.win[1](true);
        expect(wpStop).toHaveBeenCalledTimes(1);
        h.win[1](false);
        expect(wpStart).toHaveBeenCalledTimes(2);
    });

    it("suppresswhenfocused=false keeps the tone playing under focus; unset means suppress", () => {
        install();
        set("notify:sounds:suppresswhenfocused", false);
        startWaiting("blk-a");
        h.focus[1]("blk-a");
        expect(wpStop).not.toHaveBeenCalled();
        set("notify:sounds:suppresswhenfocused", undefined);
        expect(wpStop).toHaveBeenCalledTimes(1);
        set("notify:sounds:suppresswhenfocused", true);
        expect(wpStop).toHaveBeenCalledTimes(1);
        set("notify:sounds:suppresswhenfocused", false);
        expect(wpStart).toHaveBeenCalledTimes(2);
    });

    it("an app-level request is quiet while the window is in front, whichever pane has focus", () => {
        install();
        startWaiting("app:update");
        expect(wpStart).not.toHaveBeenCalled();
        expect(flashes).toHaveLength(0);
        h.win[1](false);
        expect(wpStart).toHaveBeenCalledTimes(1);
        expect(flashes).toHaveLength(0);
        h.win[1](true);
        expect(wpStop).toHaveBeenCalledTimes(1);
        set("notify:sounds:suppresswhenfocused", false);
        expect(wpStart).toHaveBeenCalledTimes(2);
    });

    it("a pane's own tone is not suspended by an app-level request's rule", () => {
        install();
        startWaiting("blk-a");
        startWaiting("app:update");
        expect(wpStart).toHaveBeenCalledTimes(1);
        h.focus[1]("blk-b");
        expect(wpStop).not.toHaveBeenCalled();
    });

    it("a tone that is already suspended is not stopped again when settings change", () => {
        install();
        startWaiting("blk-a");
        h.focus[1]("blk-a");
        expect(wpStop).toHaveBeenCalledTimes(1);
        set("notify:sounds:waiting:volume", 0.5);
        set("notify:sounds:waiting:volume", 0.6);
        expect(wpStop).toHaveBeenCalledTimes(1);
        expect(wpStart).toHaveBeenCalledTimes(1);
    });

    it("a running tone is not restarted or stopped when settings change", () => {
        install();
        startWaiting("blk-a");
        set("notify:sounds:waiting:volume", 0.5);
        set("notify:sounds:suppresswhenfocused", true);
        expect(wpStart).toHaveBeenCalledTimes(1);
        expect(wpStop).not.toHaveBeenCalled();
        expect(flashesFor("blk-a")).toBe(1);
    });

    it("a tone started under focus is suspended from the start and resumes when focus leaves", () => {
        install();
        h.focus[1]("blk-a");
        startWaiting("blk-a");
        expect(wpStart).not.toHaveBeenCalled();
        expect(flashes).toHaveLength(0);
        h.focus[1](null);
        expect(wpStart).toHaveBeenCalledTimes(1);
        expect(flashesFor("blk-a")).toBe(1);
    });

    it("only the focused pane's tone is suspended, the others keep playing", () => {
        install();
        startWaiting("blk-a");
        startWaiting("blk-b");
        h.focus[1]("blk-b");
        expect(wpStop).toHaveBeenCalledTimes(1);
        expect(wpStop.mock.contexts[0]).toBe(__getWaitingTones().get("blk-b"));
        vi.advanceTimersByTime(WAITING_LOOP_MS);
        expect(flashesFor("blk-a")).toBe(2);
        expect(flashesFor("blk-b")).toBe(1);
    });

    it("switching the tone off in the same moment focus leaves does not bring it back", () => {
        install();
        startWaiting("blk-a");
        h.focus[1]("blk-a");
        wpStart.mockClear();
        batch(() => {
            set("notify:sounds:enabled", false);
            h.focus[1](null);
        });
        expect(__getWaitingTones().size).toBe(0);
        expect(wpStart).not.toHaveBeenCalled();
    });
});

// ─── Starting a waiting tone reads the installed window focus ────────

describe("installSoundService: a waiting tone and the window focus it installed", () => {
    it("a focused pane in a focused window does not start; a blurred window does", () => {
        install();
        h.focus[1]("blk-a");
        startWaiting("blk-a");
        expect(wpStart).not.toHaveBeenCalled();
        h.win[1](false);
        startWaiting("blk-b");
        h.focus[1]("blk-b");
        expect(wpStart).toHaveBeenCalledTimes(2);
    });
});

// ─── The bus listener ────────────────────────────────────────────────

describe("installSoundService: the bus listener", () => {
    let now = 0;

    beforeEach(() => {
        now = 0;
        vi.spyOn(performance, "now").mockImplementation(() => now);
        vi.spyOn(console, "warn").mockImplementation(() => {});
    });

    it("coalesces by the sound's own window, and plays again exactly at its end", () => {
        install();
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        now = 1299;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(1);
        now = 1300;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(2);
        now = 1599;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(2);
    });

    it("a shorter window is its own: 150 ms for the accepted sound", () => {
        install();
        now = 1000;
        notify("agent.message.accepted", { sourceBlockId: "blk-1" });
        now = 1149;
        notify("agent.message.accepted", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(1);
        now = 1150;
        notify("agent.message.accepted", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(2);
    });

    it("coalesces each sound separately", () => {
        install();
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        notify("agent.turn.error", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(2);
    });

    it("a refused play does not start a window for the next one", () => {
        install();
        now = 1000;
        notify("agent.turn.complete");
        now = 1001;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("the first sound within one window of the page clock starting is dropped (the last-fired time starts at 0)", () => {
        install();
        now = 100;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).not.toHaveBeenCalled();
        now = 400;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("reads the page clock, not the wall clock", () => {
        install();
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        now = 5000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(2);
    });

    it("a refused event warns once and does not blow up the listener", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        install();
        now = 1000;
        notify("agent.waiting.for.input", { sourceBlockId: "blk-1" });
        expect(playSpy).not.toHaveBeenCalled();
        expect(warn).toHaveBeenCalledTimes(1);
        expect(String(warn.mock.calls[0][0])).toContain("refused");
        expect(flashes).toHaveLength(0);
    });

    it("a play that throws is warned about and the tab still flashes, undelayed", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        install();
        playSpy.mockImplementation(() => {
            throw new Error("no audio");
        });
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(warn).toHaveBeenCalledTimes(1);
        expect(String(warn.mock.calls[0][0])).toBe("[sound] play threw for agent.turn.complete");
        expect(flashes).toHaveLength(1);
        expect(flashes[0].blockId).toBe("blk-1");
        expect(flashes[0].delayMs).toBe(0);
    });

    it("the flash carries the sound's pattern and is held until the sound is audible", () => {
        vi.spyOn(player, "getAudioContext").mockReturnValue(fakeCtx);
        install();
        playSpy.mockReturnValue(1.2);
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(1);
        expect(flashes[0].delayMs).toBeCloseTo(audibleFlashDelayMs(fakeCtx, 1.2, 1000), 5);
        expect(flashes[0].delayMs).toBeGreaterThan(100);
    });

    it("a sound that did not play (nothing scheduled) flashes undelayed even with a context", () => {
        vi.spyOn(player, "getAudioContext").mockReturnValue(fakeCtx);
        install();
        playSpy.mockReturnValue(null);
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(1);
        expect(flashes[0].delayMs).toBe(0);
    });

    it("a sound scheduled with no context to time it by flashes undelayed", () => {
        vi.spyOn(player, "getAudioContext").mockReturnValue(null);
        install();
        playSpy.mockReturnValue(5);
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(1);
        expect(flashes[0].delayMs).toBe(0);
    });

    it("notify:tooltones:flash only turns the flash off when it is exactly false", () => {
        install();
        now = 1000;
        set("notify:tooltones:flash", undefined);
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(1);
        set("notify:tooltones:flash", true);
        now = 2000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(2);
        set("notify:tooltones:flash", false);
        now = 3000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(2);
        expect(playSpy).toHaveBeenCalledTimes(3);
    });

    it("another setting does not stand in for the flash setting", () => {
        install();
        now = 1000;
        set("notify:tooltones:enabled", false);
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(flashes).toHaveLength(1);
    });

    it("a replay-mode drop and a focus drop both skip the sound before it is coalesced", () => {
        install();
        h.focus[1]("blk-1");
        now = 1000;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).not.toHaveBeenCalled();
        h.focus[1](null);
        now = 1001;
        notify("agent.turn.complete", { sourceBlockId: "blk-1" });
        expect(playSpy).toHaveBeenCalledTimes(1);
    });
});
