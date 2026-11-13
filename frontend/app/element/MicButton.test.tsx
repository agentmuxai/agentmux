// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The pane's mic button: the level it hands its halo while this pane is the one being listened to, and
 * what it says when voice input is not set up.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

const h = vi.hoisted(() => ({ voice: {} as Record<string, unknown> }));
vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: () => h.voice }));
vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "Ctrl+M" }));

import { MicButton } from "./MicButton";

const atom = <T,>(initial: T) => {
    const [get, set] = createSignal<T>(initial);
    return Object.assign(get, { _set: (v: T) => void set(() => v) });
};
const handle = { appendFinal: () => {}, setInterim: () => {} };
let isListening: ReturnType<typeof atom<boolean>>;
let currentTargetId: ReturnType<typeof atom<string | null>>;
let lastError: ReturnType<typeof atom<string | null>>;
let level: ReturnType<typeof atom<number>>;

beforeEach(() => {
    isListening = atom(false);
    currentTargetId = atom<string | null>(null);
    lastError = atom<string | null>(null);
    level = atom(0);
    h.voice = {
        isListening,
        currentTargetId,
        lastError,
        level,
        isAvailable: () => true,
        toggleListening: vi.fn(),
        registerPane: vi.fn(),
    };
});
afterEach(() => cleanup());

const wrap = (c: HTMLElement) => c.querySelector<HTMLElement>(".mic-button-wrap")!;

describe("MicButton", () => {
    it("hands its halo the level only while this pane is the one listened to", () => {
        const { container } = render(() => <MicButton blockId="blk" handle={handle} />);
        level._set(0.6);
        expect(wrap(container).dataset.micLevel).toBe("0");
        isListening._set(true);
        currentTargetId._set("other");
        expect(wrap(container).dataset.micLevel).toBe("0");
        currentTargetId._set("blk");
        expect(wrap(container).dataset.micLevel).toBe("0.6");
        expect(wrap(container).style.getPropertyValue("--mic-level")).toBe("0.6");
        level._set(0.15);
        expect(wrap(container).style.getPropertyValue("--mic-level")).toBe("0.15");
        isListening._set(false);
        expect(wrap(container).style.getPropertyValue("--mic-level")).toBe("0");
    });

    it("shows no level with an engine that reports none", () => {
        delete h.voice.level;
        isListening._set(true);
        currentTargetId._set("blk");
        const { container } = render(() => <MicButton blockId="blk" handle={handle} />);
        expect(wrap(container).dataset.micLevel).toBe("0");
    });

    it("says voice input isn't set up, and where to set it up, when a start was refused for it", () => {
        lastError._set("not-configured");
        const { container } = render(() => <MicButton blockId="blk" handle={handle} />);
        expect(wrap(container).classList.contains("mic-blocked")).toBe(true);
        const title = container.querySelector("button")!.getAttribute("title")!;
        expect(title).toContain("isn't set up");
        expect(title).toContain("Settings > Recording");
    });
});
