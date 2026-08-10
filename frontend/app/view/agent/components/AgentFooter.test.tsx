// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Esc-clear / Undo regression tests for AgentFooter (reagent P1 / codex P2
 * on PR #2497): `escClearedDraft` must be invalidated by any edit or send
 * after the Esc-clear, so Ctrl/Cmd+Z can't resurrect stale text once a new
 * message has been typed, deleted back to empty, or sent.
 *
 * Also covers the ghost-text next-prompt suggestion's restore-on-clear
 * behavior added by
 * docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentFooter } from "./AgentFooter";
import { ObjectService } from "@/app/store/services";
import type { AgentViewModel } from "../agent-model";

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

// Minimal AgentViewModel double — only the fields AgentFooter actually reads:
// blockId (voice-target wiring), blockAtom (ghost-text suggestion meta), and
// voiceTargetRef (onMount registers a PaneVoiceHandle onto it unconditionally
// whenever a viewModel is present). suggestion is fixed for the lifetime of
// the mock, which is deliberate: these tests exist to prove editing the
// composer never triggers a write that would clear it, not to simulate the
// real reactive meta atom. `gen` mirrors term:next_prompt_suggestion_gen —
// must be a real number whenever `suggestion` is set, or the placeholder
// memo's initial "undefined !== undefined" comparison would wrongly
// suppress it from the very first render (see suggestionGenMaskedAtSend's
// doc comment in AgentFooter.tsx).
function makeViewModel(suggestion: string | undefined, gen = 1): AgentViewModel {
    return {
        blockId: "test-block",
        blockAtom: () =>
            ({
                meta: {
                    "term:next_prompt_suggestion": suggestion,
                    "term:next_prompt_suggestion_gen": suggestion ? gen : undefined,
                },
            }) as any,
        voiceTargetRef: { current: null },
    } as unknown as AgentViewModel;
}

// Reactive variant for tests that need meta to actually change after
// mount (simulating useNextPromptSuggestion.ts's guard-1 clear or a later
// turn's fresh write landing).
function makeReactiveViewModel(initial: { suggestion: string | undefined; gen: number | undefined }) {
    const [state, setState] = createSignal(initial);
    const vm = {
        blockId: "test-block",
        blockAtom: () =>
            ({
                meta: {
                    "term:next_prompt_suggestion": state().suggestion,
                    "term:next_prompt_suggestion_gen": state().gen,
                },
            }) as any,
        voiceTargetRef: { current: null },
    } as unknown as AgentViewModel;
    return { vm, setState };
}

describe("AgentFooter ghost-text next-prompt suggestion (SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md)", () => {
    it("shows the suggestion as placeholder text when set", () => {
        render(() => <AgentFooter agentName="Test" viewModel={makeViewModel("Run the tests")} />);
        expect(screen.getByPlaceholderText("Run the tests")).toBeTruthy();
    });

    // The actual bug this spec fixes: typing over the suggestion then
    // deleting back to empty used to permanently clear it from block meta,
    // so the composer fell back to "Send message to <agent>..." instead of
    // showing the suggestion again.
    it("keeps showing the same suggestion after typing over it and deleting back to empty", async () => {
        const updateSpy = vi.spyOn(ObjectService, "UpdateObjectMeta").mockResolvedValue(undefined);
        render(() => <AgentFooter agentName="Test" viewModel={makeViewModel("Run the tests")} />);
        const user = userEvent.setup();
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        await user.type(ta, "actually let me refactor first");
        await user.clear(ta);

        expect(ta.value).toBe("");
        expect(ta.placeholder).toBe("Run the tests");
        // The regression: handleInput used to null term:next_prompt_suggestion
        // on the first keystroke into an empty box. Editing must never write
        // to it at all — only a new turn starting or session end may.
        expect(updateSpy).not.toHaveBeenCalled();
    });

    it("Tab accepts the suggestion into the composer without clearing it from meta", async () => {
        const updateSpy = vi.spyOn(ObjectService, "UpdateObjectMeta").mockResolvedValue(undefined);
        render(() => <AgentFooter agentName="Test" viewModel={makeViewModel("Run the tests")} />);
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        keyOn(ta, "Tab");
        expect(ta.value).toBe("Run the tests");
        expect(updateSpy).not.toHaveBeenCalled();

        // Deleting the accepted text back to empty shows the same
        // suggestion again — accepting via Tab and typing it by hand are
        // treated identically (spec §5 point 1).
        const user = userEvent.setup();
        await user.clear(ta);
        expect(ta.placeholder).toBe("Run the tests");
    });

    it("falls back to the default placeholder when no suggestion is set", () => {
        render(() => <AgentFooter agentName="Test" viewModel={makeViewModel(undefined)} />);
        expect(screen.getByPlaceholderText("Send message to Test...")).toBeTruthy();
    });

    // Reagentx P1 on #2515: handleSend synchronously empties the composer,
    // but the previous turn's stale suggestion is only cleared from meta by
    // an async fire-and-forget RPC (useNextPromptSuggestion.ts guard 1) —
    // this pins the worst case, where that RPC hasn't landed by the time
    // the placeholder next renders, using a viewModel whose blockAtom never
    // updates at all (simulating an arbitrarily slow/never-resolving clear).
    it("does not flash the previous turn's stale suggestion in the now-empty box right after sending", async () => {
        const onSendMessage = vi.fn();
        render(() => (
            <AgentFooter agentName="Test" viewModel={makeViewModel("Run the tests")} onSendMessage={onSendMessage} />
        ));
        const user = userEvent.setup();
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        await user.type(ta, "let's refactor instead");
        keyOn(ta, "Enter");

        expect(onSendMessage).toHaveBeenCalledWith("let's refactor instead");
        expect(ta.value).toBe("");
        expect(ta.placeholder).toBe("Send message to Test...");
    });

    it("shows a genuinely new suggestion normally once meta actually updates after send", async () => {
        const { vm, setState } = makeReactiveViewModel({ suggestion: "Run the tests", gen: 1 });
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" viewModel={vm} onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        await user.type(ta, "let's refactor instead");
        keyOn(ta, "Enter");
        expect(ta.placeholder).toBe("Send message to Test...");

        // Simulates useNextPromptSuggestion.ts guard 1's clear RPC landing,
        // then a later turn's fresh (differently-worded) suggestion
        // arriving — the mask must not shadow it.
        setState({ suggestion: undefined, gen: 2 });
        setState({ suggestion: "Check the logs", gen: 3 });
        expect(ta.placeholder).toBe("Check the logs");
    });

    // Reagentx P1 on #2515, second round: an earlier version of this fix
    // compared the suggestion TEXT masked at send against the live text —
    // if a later turn's genuinely fresh suggestion happened to be the exact
    // same string (a plausible repeat, e.g. Haiku predicting "Run the
    // tests" twice), that comparison would wrongly suppress a legitimate
    // current suggestion. Only the generation counter, not the text, is
    // compared now — this pins the collision directly.
    it("shows a fresh suggestion after send even when its text is identical to the one masked", async () => {
        const { vm, setState } = makeReactiveViewModel({ suggestion: "Run the tests", gen: 1 });
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" viewModel={vm} onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        await user.type(ta, "ok will do");
        keyOn(ta, "Enter");
        expect(ta.placeholder).toBe("Send message to Test...");

        // Same text, but a genuinely new write (fresh generation) — must
        // show, not stay suppressed just because the string happens to match.
        setState({ suggestion: "Run the tests", gen: 2 });
        expect(ta.placeholder).toBe("Run the tests");
    });
});
