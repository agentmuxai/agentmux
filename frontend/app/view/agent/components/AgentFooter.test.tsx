// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Esc-clear / Undo regression tests for AgentFooter (reagent P1 / codex P2
 * on PR #2497): `escClearedDraft` must be invalidated by any edit or send
 * after the Esc-clear, so Ctrl/Cmd+Z can't resurrect stale text once a new
 * message has been typed, deleted back to empty, or sent.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentFooter } from "./AgentFooter";

afterEach(() => {
    cleanup();
});

function keyOn(el: Element, key: string, opts: KeyboardEventInit = {}): void {
    el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...opts }));
}

function getComposer(): HTMLTextAreaElement {
    return screen.getByPlaceholderText(/Send message to/) as HTMLTextAreaElement;
}

describe("AgentFooter Esc-clear Undo", () => {
    it("Ctrl+Z restores the draft right after an Esc-clear", async () => {
        render(() => <AgentFooter agentName="Test" />);
        const user = userEvent.setup();
        const ta = getComposer();

        await user.click(ta);
        await user.type(ta, "hello");
        keyOn(ta, "Escape");
        expect(ta.value).toBe("");

        keyOn(ta, "z", { ctrlKey: true });
        expect(ta.value).toBe("hello");
    });

    it("does not resurrect a stale Esc-cleared draft after typing something new and deleting it back to empty", async () => {
        render(() => <AgentFooter agentName="Test" />);
        const user = userEvent.setup();
        const ta = getComposer();

        await user.click(ta);
        await user.type(ta, "hello");
        keyOn(ta, "Escape");
        expect(ta.value).toBe("");

        await user.type(ta, "B");
        await user.type(ta, "{Backspace}");
        expect(ta.value).toBe("");

        // Bug (reagent P1 / codex P2): escClearedDraft was never invalidated
        // by this new edit, so Ctrl+Z would resurrect the stale "hello"
        // instead of falling through to native undo.
        keyOn(ta, "z", { ctrlKey: true });
        expect(ta.value).toBe("");
    });

    it("does not resurrect a stale Esc-cleared draft after sending a new message", async () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();

        await user.click(ta);
        await user.type(ta, "hello");
        keyOn(ta, "Escape");
        expect(ta.value).toBe("");

        await user.type(ta, "world");
        keyOn(ta, "Enter");
        expect(onSendMessage).toHaveBeenCalledWith("world");
        expect(ta.value).toBe("");

        // Bug (reagent P1): handleSend never invalidated escClearedDraft, so
        // Ctrl+Z after sending would resurrect the pre-Esc "hello" draft.
        keyOn(ta, "z", { ctrlKey: true });
        expect(ta.value).toBe("");
    });
});
