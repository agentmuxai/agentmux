// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The in-page harness's typing safety, in jsdom: the draft snapshot, the
// stray-key guard, and restore — including the case Codex found on #3569,
// where the footer remounts mid-window (a pane-stack tab switch) and the new
// composer shows the synthetic text AgentFooter persisted per block.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const SRC = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "bench-page.js"), "utf8");

beforeEach(() => {
    if (!globalThis.CSS) globalThis.CSS = {};
    if (!CSS.escape) CSS.escape = (s) => String(s).replace(/["\\]/g, "\\$&");
    delete window.__fcb;
    document.body.innerHTML = "";
    window.eval(SRC);
});
afterEach(() => vi.useRealTimers());

/** A block frame with a composer, as blockframe.tsx + AgentFooter render it. */
function pane(blockId, value = "") {
    const frame = document.createElement("div");
    frame.setAttribute("data-blockid", blockId);
    const ta = document.createElement("textarea");
    ta.className = "agent-input";
    ta.value = value;
    frame.appendChild(ta);
    document.body.appendChild(frame);
    return { frame, ta };
}

/** Replace the composer, carrying over its current value — what a footer
 *  remount does via composerDrafts. Returns the new element. */
function remount(frame) {
    const old = frame.querySelector("textarea.agent-input");
    const ta = document.createElement("textarea");
    ta.className = "agent-input";
    ta.value = old.value;
    old.remove();
    frame.appendChild(ta);
    return ta;
}

describe("typing target", () => {
    it("restores the draft and selection on the captured composer", async () => {
        const { ta } = pane("b1", "my draft");
        ta.setSelectionRange(3, 5);
        window.__fcb.armTyping("b1");
        ta.value += "ffff";
        const inputs = vi.fn();
        ta.addEventListener("input", inputs);
        const r = await window.__fcb.restoreTyping();
        expect(r).toMatchObject({ ok: true, how: "captured composer", stray: 0 });
        expect(ta.value).toBe("my draft");
        expect([ta.selectionStart, ta.selectionEnd]).toEqual([3, 5]);
        // The bubbled input event is what makes AgentFooter persist the draft.
        expect(inputs).toHaveBeenCalledTimes(1);
    });

    it("restores through the remounted composer when the captured one is detached", async () => {
        const { frame, ta } = pane("b1", "keep me");
        window.__fcb.armTyping("b1");
        ta.value += "fff";
        const fresh = remount(frame);
        expect(fresh.value).toBe("keep mefff");
        const inputs = vi.fn();
        fresh.addEventListener("input", inputs);
        expect(window.__fcb.typingOk()).toBe(false);
        const r = await window.__fcb.restoreTyping();
        expect(r).toMatchObject({ ok: true, how: "remounted composer" });
        expect(fresh.value).toBe("keep me");
        expect(inputs).toHaveBeenCalledTimes(1);
    });

    it("waits for a composer that is not mounted yet, then restores it", async () => {
        vi.useFakeTimers();
        const { frame, ta } = pane("b1", "later");
        window.__fcb.armTyping("b1");
        ta.value += "ff";
        ta.remove();
        const p = window.__fcb.restoreTyping();
        await vi.advanceTimersByTimeAsync(2_000);
        const back = document.createElement("textarea");
        back.className = "agent-input";
        back.value = "laterff";
        frame.appendChild(back);
        await vi.advanceTimersByTimeAsync(200);
        await expect(p).resolves.toMatchObject({ ok: true, how: "remounted composer" });
        expect(back.value).toBe("later");
    });

    it("gives the draft back when no composer returns within 30 s", async () => {
        vi.useFakeTimers();
        const { frame } = pane("b1", "precious text");
        window.__fcb.armTyping("b1");
        frame.remove();
        const p = window.__fcb.restoreTyping();
        await vi.advanceTimersByTimeAsync(31_000);
        await expect(p).resolves.toEqual({ ok: false, blockId: "b1", draft: "precious text", stray: 0 });
    });

    it("swallows and counts a key aimed anywhere but the captured composer", async () => {
        const { ta } = pane("b1");
        const other = pane("b2").ta;
        window.__fcb.armTyping("b1");
        const onOther = vi.fn();
        other.addEventListener("keydown", onOther);
        const ev = new KeyboardEvent("keydown", { key: "f", bubbles: true, cancelable: true });
        other.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(true);
        expect(onOther).not.toHaveBeenCalled();
        const own = new KeyboardEvent("keydown", { key: "f", bubbles: true, cancelable: true });
        ta.dispatchEvent(own);
        expect(own.defaultPrevented).toBe(false);
        expect(window.__fcb.typingOk()).toBe(false); // one stray key already
        const r = await window.__fcb.restoreTyping();
        expect(r.stray).toBe(1);
        // The guard is gone once restored.
        const after = new KeyboardEvent("keydown", { key: "f", bubbles: true, cancelable: true });
        other.dispatchEvent(after);
        expect(after.defaultPrevented).toBe(false);
    });

    it("refuses a block without a composer", () => {
        pane("b1");
        expect(() => window.__fcb.armTyping("nope")).toThrow(/no composer in block nope/);
    });

    it("restore with nothing armed is a no-op", async () => {
        await expect(window.__fcb.restoreTyping()).resolves.toMatchObject({ ok: true, how: "nothing to restore" });
    });
});

describe("visibility", () => {
    it("a window cannot start while the page is hidden", () => {
        vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
        expect(() => window.__fcb.startWindow({ streamKb: 1, secs: 1 })).toThrow(/hidden at window start/);
    });
});
