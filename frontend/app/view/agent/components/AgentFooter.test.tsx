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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentFooter, AgentWorkingRow } from "./AgentFooter";
import { recordCompactionSample } from "../compaction-estimate";
import type { TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import { ObjectService } from "@/app/store/services";
import type { AgentViewModel } from "../agent-model";
import { requestComposerFocus } from "../composer-focus";
import { focusManager } from "@/app/store/focusManager";

/** Run the clock just until the Working row's line has finished typing out
 *  (it stops changing), so a test reads the whole line without letting the
 *  row's other timing rules move on. Needs fake timers. A turn's first line
 *  types out too (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10). */
function finishTyping(container: HTMLElement): void {
    const text = () => container.querySelector(".agent-working-row-left")?.textContent;
    let prev = text();
    for (let i = 0; i < 400; i++) {
        vi.advanceTimersByTime(28);
        const now = text();
        if (now === prev) return;
        prev = now;
    }
}
const FAKE_CLOCK: Parameters<typeof vi.useFakeTimers>[0] = {
    toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"],
};

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

/**
 * Regression test: the composer textarea is a flex sibling of the chat
 * scroll region, not a descendant of it (see the PageUp/PageDown branch's
 * own comment in handleKeyDown). Left unprevented, Page Up/Down's browser
 * default escapes past the pane entirely to scroll an unrelated ancestor,
 * which reads as the whole chat pane jumping off screen.
 */
describe("AgentFooter PageUp/PageDown", () => {
    it("prevents the default scroll for PageUp and PageDown while the composer is focused", async () => {
        render(() => <AgentFooter agentName="Test" />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);

        const pageUp = new KeyboardEvent("keydown", { key: "PageUp", bubbles: true, cancelable: true });
        ta.dispatchEvent(pageUp);
        expect(pageUp.defaultPrevented).toBe(true);

        const pageDown = new KeyboardEvent("keydown", { key: "PageDown", bubbles: true, cancelable: true });
        ta.dispatchEvent(pageDown);
        expect(pageDown.defaultPrevented).toBe(true);
    });

    it("does not prevent default when the composer itself is overflowing and has its own paging to do (codex P2 on PR #3190)", async () => {
        render(() => <AgentFooter agentName="Test" />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);

        // jsdom never computes real layout, so scrollHeight/clientHeight are
        // both 0 by default — simulate the "draft grew past the 200px cap"
        // state (_pending-footer.scss's `max-height: 200px; overflow-y: auto`)
        // the same way a long pasted prompt would trigger it for real.
        Object.defineProperty(ta, "scrollHeight", { value: 400, configurable: true });
        Object.defineProperty(ta, "clientHeight", { value: 200, configurable: true });

        const pageUp = new KeyboardEvent("keydown", { key: "PageUp", bubbles: true, cancelable: true });
        ta.dispatchEvent(pageUp);
        expect(pageUp.defaultPrevented).toBe(false);
    });
});

/**
 * Regression tests for
 * docs/specs/SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11.md.
 *
 * jsdom has no layout engine, so `offsetTop` (what the old mirror-div
 * `caretVisualEdge` measured) always reads 0 regardless of actual cursor
 * position — meaning the pre-fix code's "on visual row 0" check would have
 * been unconditionally true for any position with no `\n` before it in this
 * test environment, unable to meaningfully exercise the bug at all. The fix
 * (require the true start/end of content via `selectionStart`/`selectionEnd`,
 * no layout measurement) is fully testable here precisely because it no
 * longer depends on layout.
 */
describe("AgentFooter composer history vs. selection (SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11.md)", () => {
    async function sendMessages(ta: HTMLTextAreaElement, user: ReturnType<typeof userEvent.setup>, ...messages: string[]) {
        for (const msg of messages) {
            await user.type(ta, msg);
            keyOn(ta, "Enter");
        }
    }

    it("does not trigger history recall while a Shift+ArrowUp selection has only partially reached the top line", async () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "first message", "second message");

        const draft = "hello world\nsecond line";
        await user.type(ta, draft);
        expect(ta.value).toBe(draft);

        // Selection extended backward (upward) from position 18 (on line 2)
        // to position 5 (mid-way through "hello world" on line 1) — the
        // normal, expected result of a Shift+ArrowUp sequence reaching the
        // top line without yet covering it fully.
        ta.setSelectionRange(5, 18, "backward");
        keyOn(ta, "ArrowUp", { shiftKey: true });

        // Bug: the old "on visual row 0" check would have fired history
        // recall here, replacing the draft. Fixed: the active selection edge
        // (selectionStart=5, backward direction) isn't at true position 0
        // yet, so the draft and selection must be left alone.
        expect(ta.value).toBe(draft);
        expect(ta.selectionStart).toBe(5);
        expect(ta.selectionEnd).toBe(18);
    });

    it("triggers history recall on Shift+ArrowUp once the selection reaches true position 0", async () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "first message", "second message");

        const draft = "hello world\nsecond line";
        await user.type(ta, draft);

        // Now the selection covers the ENTIRE top line (the state one more
        // Shift+ArrowUp should have produced from the partial-selection case
        // above).
        ta.setSelectionRange(0, 18, "backward");
        keyOn(ta, "ArrowUp", { shiftKey: true });

        expect(ta.value).toBe("second message"); // most recently sent
    });

    it("does not trigger history recall on plain ArrowUp mid-line, only once the caret reaches true position 0", async () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "only message");

        const draft = "hello world";
        await user.type(ta, draft);

        ta.setSelectionRange(5, 5); // collapsed cursor mid-line, not at start
        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe(draft); // untouched — jsdom doesn't move the caret itself

        ta.setSelectionRange(0, 0); // now truly at the start
        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("only message");
    });

    it("uses the active (moving) selection edge, not always selectionStart, for a forward-direction selection", async () => {
        // reagentx P2 on #2522's sibling finding, §2.3 of the spec: a
        // forward-direction selection's moving end for Shift+ArrowUp is
        // selectionEnd, not selectionStart. The old code always read
        // selectionStart regardless of direction — here selectionStart is 0
        // but the ACTUAL moving end (selectionEnd=5) is not, so history must
        // NOT fire even though selectionStart alone would suggest it should.
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "only message");

        const draft = "hello world";
        await user.type(ta, draft);

        ta.setSelectionRange(0, 5, "forward");
        keyOn(ta, "ArrowUp", { shiftKey: true });

        expect(ta.value).toBe(draft); // must not have been replaced
    });

    it("requires the true end of a MULTI-LINE recalled entry before advancing, but accepts position 0 of a SINGLE-LINE one", async () => {
        // reagent P1: position 0 is not "nowhere left to move" for a
        // multi-line entry — line two is still sitting right below it, and
        // native ArrowDown must be able to reach it. So ArrowDown only
        // treats position 0 as a valid trigger when the currently-displayed
        // entry is single-line (no embedded "\n"); a multi-line entry still
        // requires the true end, exactly like before this feature's fix.
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "line one\nline two", "second message");

        // Walk back to the oldest (multiline) entry.
        ta.setSelectionRange(0, 0);
        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("second message");
        keyOn(ta, "ArrowUp"); // caret already at 0 from the prior recall
        expect(ta.value).toBe("line one\nline two");

        // At true position 0 of this MULTI-LINE entry: line two is still
        // below the caret, so ArrowDown must NOT advance — it needs to stay
        // available for native down-line movement instead.
        keyOn(ta, "ArrowDown");
        expect(ta.value).toBe("line one\nline two"); // untouched

        // Only once truly at the end of the (multi-line) content does it
        // advance — landing on the next entry with the caret at ITS front.
        ta.setSelectionRange("line one\nline two".length, "line one\nline two".length);
        keyOn(ta, "ArrowDown");
        expect(ta.value).toBe("second message");
        expect(ta.selectionStart).toBe(0);
        expect(ta.selectionEnd).toBe(0);

        // "second message" is SINGLE-LINE: position 0 (where the caret was
        // just placed) has nothing below it — there's no second line to
        // strand — so ArrowDown advances immediately, no extra keypress
        // needed to reach a true end that wouldn't add any information here.
        keyOn(ta, "ArrowDown");
        expect(ta.value).toBe(""); // past the newest entry — the stashed (empty) live draft
    });

    it("lands the caret at the front for each interim ArrowDown step, and at the end only when returning to the live draft", async () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "first message", "second message", "third message");

        await user.type(ta, "my draft");
        ta.setSelectionRange(0, 0); // as if the user pressed Home first

        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("third message");
        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("second message");

        // Coming back down: each interim step lands at the front too, so a
        // repeated ArrowDown steps forward continuously just like ArrowUp.
        keyOn(ta, "ArrowDown");
        expect(ta.value).toBe("third message");
        expect(ta.selectionStart).toBe(0);
        expect(ta.selectionEnd).toBe(0);

        // ...except the final step, past the newest entry back to the live
        // draft — that one lands at the END, ready to keep typing where the
        // user left off.
        keyOn(ta, "ArrowDown");
        expect(ta.value).toBe("my draft");
        expect(ta.selectionStart).toBe("my draft".length);
        expect(ta.selectionEnd).toBe("my draft".length);
    });

    it("lands the caret at position 0 after an ArrowUp recall, so a repeated ArrowUp immediately steps back again", async () => {
        // Before this fix, setComposerValue always parked the caret at the
        // END of the recalled text. Since ArrowUp only navigates further
        // back when the caret is already at true position 0, a second
        // consecutive ArrowUp press (with no manual caret reset in between,
        // unlike the other tests in this file) would just move the caret
        // back to 0 natively instead of recalling the next-older entry —
        // requiring two ArrowUp presses per history step for a single-line
        // message. Every other test here works around that by explicitly
        // calling ta.setSelectionRange(0, 0) between presses; this test
        // deliberately does not, to prove that workaround is no longer
        // necessary.
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.click(ta);
        await sendMessages(ta, user, "first message", "second message", "third message");

        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("third message");
        expect(ta.selectionStart).toBe(0);
        expect(ta.selectionEnd).toBe(0);

        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("second message");
        expect(ta.selectionStart).toBe(0);

        keyOn(ta, "ArrowUp");
        expect(ta.value).toBe("first message");
        expect(ta.selectionStart).toBe(0);
    });
});

// Minimal AgentViewModel double — only the fields AgentFooter actually reads:
// blockId (voice-target wiring), blockAtom (ghost-text suggestion meta), and
// voiceTargetRef / focusTargetRef (onMount registers a PaneVoiceHandle and
// the textarea onto them unconditionally whenever a viewModel is present).
// suggestion is fixed for the lifetime of
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
        focusTargetRef: { current: null },
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
        focusTargetRef: { current: null },
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

    // The placeholder hides the previous turn's suggestion at send, but its
    // clear is an async write: until it lands, the meta still holds the text.
    // Tab and → must offer what the placeholder shows, not the raw meta.
    it("Tab and → don't insert a suggestion hidden at send while its clear is still pending", async () => {
        const onSendMessage = vi.fn();
        render(() => (
            <AgentFooter agentName="Test" viewModel={makeViewModel("Run the tests")} onSendMessage={onSendMessage} />
        ));
        const user = userEvent.setup();
        const ta = screen.getByRole("textbox") as HTMLTextAreaElement;

        await user.type(ta, "let's refactor instead");
        keyOn(ta, "Enter");
        expect(ta.placeholder).toBe("Send message to Test...");

        keyOn(ta, "Tab");
        expect(ta.value).toBe("");
        keyOn(ta, "ArrowRight");
        expect(ta.value).toBe("");
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

// SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.4:
// composerDrafts is module-level, keyed by blockId — every case below uses
// a blockId dedicated to that case (never "test-block", which every other
// describe block in this file shares) so these tests can't leak draft
// state into, or read stale state left by, unrelated tests sharing this
// file's module graph.
function makeDraftViewModel(blockId: string): AgentViewModel {
    return {
        blockId,
        blockAtom: () => ({ meta: {} }) as any,
        voiceTargetRef: { current: null },
        focusTargetRef: { current: null },
    } as unknown as AgentViewModel;
}

describe("AgentFooter composer draft persistence (SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.4)", () => {
    it("restores a typed draft after this block's footer unmounts and remounts (the tab-switch-away/back shape)", async () => {
        const vm = makeDraftViewModel("draft-block-remount");
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        const user = userEvent.setup();
        await user.type(getComposer(), "half-written thought");
        unmount();

        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(getComposer().value).toBe("half-written thought");
    });

    it("does not leak a draft across two different blockIds", async () => {
        const vmA = makeDraftViewModel("draft-block-a");
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={vmA} />);
        const user = userEvent.setup();
        await user.type(getComposer(), "block A's draft");
        unmount();

        const vmB = makeDraftViewModel("draft-block-b");
        render(() => <AgentFooter agentName="Test" viewModel={vmB} />);
        expect(getComposer().value).toBe("");
    });

    it("clears the persisted draft on send — a later remount starts blank, not with the sent text", async () => {
        const onSendMessage = vi.fn();
        const vm = makeDraftViewModel("draft-block-sent");
        const { unmount } = render(() => (
            <AgentFooter agentName="Test" viewModel={vm} onSendMessage={onSendMessage} />
        ));
        const user = userEvent.setup();
        const ta = getComposer();
        await user.type(ta, "ship it");
        keyOn(ta, "Enter");
        expect(onSendMessage).toHaveBeenCalledWith("ship it");
        unmount();

        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(getComposer().value).toBe("");
    });

    it("an Esc-clear also clears the persisted draft — a later remount does not resurrect the pre-clear text", async () => {
        const vm = makeDraftViewModel("draft-block-escclear");
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        const user = userEvent.setup();
        const ta = getComposer();
        await user.type(ta, "changed my mind");
        keyOn(ta, "Escape");
        expect(ta.value).toBe("");
        unmount();

        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(getComposer().value).toBe("");
    });

    it("without a viewModel, draft persistence is a no-op — no crash, nothing restored", async () => {
        const { unmount } = render(() => <AgentFooter agentName="Test" />);
        const user = userEvent.setup();
        await user.type(getComposer(), "no blockId to key off");
        unmount();

        render(() => <AgentFooter agentName="Test" />);
        expect(getComposer().value).toBe("");
    });
});

/**
 * Reconnecting/Compacting sub-states, relocated onto AgentWorkingRow from
 * AgentComposerStrip's now-removed rightText() display (2026-08-27, Part 2
 * of docs/specs/SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md §10).
 * Both states must render the row even with `loading={false}` and no
 * `sessionStats` — that's the whole point of the relocation (a stale-resume
 * reconnect fires only after the underlying process has already exited, so
 * `loading` is typically false while it's in progress).
 */
describe("AgentWorkingRow compacting/reconnecting sub-states (SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md Part 2)", () => {
    it('renders "Reconnecting…" in the left zone and an elapsed counter in the right zone, even with loading=false and no sessionStats', () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={false} reconnecting={{ startedAt: Date.now() }} />
        ));

        expect(container.querySelector(".agent-working-row--loading")).toBeTruthy();
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Reconnecting…");
        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^\d+s$/);
    });

    it('renders "Compacting…" similarly when compacting is set (and reconnecting is not)', () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={{ trigger: "auto", startedAt: Date.now() }} />
        ));

        expect(container.querySelector(".agent-working-row--loading")).toBeTruthy();
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Compacting…");
        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^\d+s$/);
    });

    it("reconnecting takes priority over compacting when both are somehow set", () => {
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                compacting={{ trigger: "auto", startedAt: Date.now() }}
                reconnecting={{ startedAt: Date.now() }}
            />
        ));

        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Reconnecting…");
    });

    it("is not visible (no worked-summary/nothing) when neither loading, compacting, nor reconnecting is set and there's no sessionStats", () => {
        const { container } = render(() => <AgentWorkingRow loading={false} />);

        expect(container.querySelector(".agent-working-row")).toBeNull();
    });
});

/**
 * SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md: the loading row shows the
 * pane's ambient summary instead of "tool · arg", reports the turn's output as
 * one counter (SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md),
 * and has no shimmer overlay.
 */
describe("AgentWorkingRow ambient summary and per-turn tokens", () => {
    beforeEach(() => {
        vi.useFakeTimers(FAKE_CLOCK);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("shows the ambient summary in the left zone", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="  Fix the login redirect loop " />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Fix the login redirect loop");
    });

    it("falls back to the cycling phrase when there is no summary", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} activitySummary={null} />);
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toMatch(/…$/);
    });

    it("lets a status win over the summary", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} stopping={true} activitySummary="Fix the login redirect loop" />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Stopping…");
    });

    // SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md: one
    // number, the turn's output, as Claude Code shows it.
    it("shows the turn's output as one counter, ↓ while the model streams, next to the elapsed time", () => {
        const { container } = render(() => (
            <AgentWorkingRow
                loading={true}
                activitySummary="Fix the login redirect loop"
                // 180k of context re-sent on the last call: input, not counted.
                turnTokens={{ input: 180_000, output: 200, outputDone: 1_000 }}
            />
        ));

        const right = container.querySelector(".agent-working-row-right")?.textContent ?? "";
        expect(right).toMatch(/^↓ 1\.2k tokens {2}·  \d+s$/);
        expect(right).not.toContain("180k");
    });

    it("shows ↑ while a request is in flight", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} turnTokens={{ input: 5_000, output: 300, requesting: true }} />
        ));

        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^↑ 300 tokens {2}·  \d+s$/);
    });

    it("counts a call still streaming from its characters, at four a token", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} turnTokens={{ input: 5_000, output: 0, outputDone: 100, streamedChars: 400 }} />
        ));

        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^↓ 200 tokens/);
    });

    it("shows no token counter before the turn's first call", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} turnTokens={null} />);

        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^\d+s$/);
    });

    it("the Worked summary shows that turn's output, with no input figure", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={false} sessionStats={{ input_tokens: 840_000, output_tokens: 2_400, duration_ms: 42_000 }} />
        ));

        const left = container.querySelector(".agent-working-row-left")?.textContent ?? "";
        expect(left).toBe("✓ Worked  ·  42s  ·  2.4k tokens");
        expect(left).not.toContain("840k");
    });

    it("the Worked summary leaves the token count out when the turn reported none", () => {
        const { container } = render(() => <AgentWorkingRow loading={false} sessionStats={{ duration_ms: 42_000 }} />);

        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("✓ Worked  ·  42s");
    });

    it("renders one solid-color left zone: no shimmer or typing overlay classes", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} activitySummary="Fix it" />);

        const left = container.querySelector(".agent-working-row-left") as HTMLElement;
        expect(left.className).toBe("agent-working-row-left");
    });
});

// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10: the presenter's
// dwell counts from when a line is fully printed, by the wall clock. The row's
// type-out must follow the same clock, or a throttled timer (a background
// window) leaves the line half printed when the dwell thinks it is done.
describe("AgentWorkingRow type-out follows the clock", () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("a throttled timer catches up: one late callback prints the whole line", () => {
        vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
        const [needsYou, setNeedsYou] = createSignal<string | null>(null);
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="Fix the login redirect loop" needsYou={needsYou()} />
        ));
        const primary = () => container.querySelector(".agent-working-row-primary")?.textContent;
        finishTyping(container);
        expect(primary()).toBe("Fix the login redirect loop"); // a turn's first line types out too
        setNeedsYou("Waiting for your approval: git push"); // a rank change: types out
        expect(primary()).toBe("");
        // The window was in the background: 2 s pass, and one callback runs.
        vi.setSystemTime(Date.now() + 2_000);
        vi.advanceTimersByTime(30);
        expect(primary()).toBe("Waiting for your approval: git push");
    });

    // Reported live: a line printed, then reset and printed again, about once
    // a second. The status memo fires every tick (a new object each time) and
    // `on()` doesn't compare, so the type-out restarted on the same key.
    it("a line on screen is never typed out again while it stays the same line", () => {
        vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
        const [needsYou, setNeedsYou] = createSignal<string | null>(null);
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="Fix the login redirect loop" needsYou={needsYou()} />
        ));
        const primary = () => container.querySelector(".agent-working-row-primary")?.textContent;
        setNeedsYou("Waiting for your approval: git push");
        vi.advanceTimersByTime(2_000); // fully typed out
        expect(primary()).toBe("Waiting for your approval: git push");
        // Five seconds of ticks: the line must stay whole at every step.
        for (let t = 0; t < 5_000; t += 100) {
            vi.advanceTimersByTime(100);
            expect(primary()).toBe("Waiting for your approval: git push");
        }
    });

    // The operator's call: every new step line types out, not only a change
    // of kind (a second tool after a first used to appear whole).
    it("a new step line of the same kind types out too", () => {
        vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
        const step = (label: string, id: string) => ({
            phase: null,
            phaseSince: 0,
            tools: [{ id, activity: { family: "bash" as const, label }, startedAt: Date.now() - 3_000 }],
            plan: null,
            planAt: 0,
        });
        const [activity, setActivity] = createSignal(step("Running the srv tests", "t1"));
        const { container } = render(() => <AgentWorkingRow loading={true} activitySummary="Fix it" activity={activity()} />);
        const primary = () => container.querySelector(".agent-working-row-primary")?.textContent;
        vi.advanceTimersByTime(5_000); // the first step is on screen and past its dwell
        setActivity(step("Building the frontend", "t2"));
        expect(primary()).toBe(""); // starts typing, not whole
        vi.advanceTimersByTime(10 * 28 + 5);
        expect(primary()).toBe("Building t"); // ten characters in
        vi.advanceTimersByTime(2_000);
        expect(primary()).toBe("Building the frontend");
    });

    // The operator's call (2026-10-10): a turn's first line types out too; it
    // had shown whole since 2026-08-10.
    it("when a turn starts, its first line types out", () => {
        vi.useFakeTimers(FAKE_CLOCK);
        const [loading, setLoading] = createSignal(false);
        const { container } = render(() => <AgentWorkingRow loading={loading()} activitySummary="Fix the login redirect loop" />);
        const primary = () => container.querySelector(".agent-working-row-primary")?.textContent;
        vi.advanceTimersByTime(5_000); // idle a while: the presenter has seen this line
        setLoading(true);
        expect(primary()).toBe(""); // not whole at once
        vi.advanceTimersByTime(3 * 28 + 5);
        expect(primary()).toBe("Fix");
        finishTyping(container);
        expect(primary()).toBe("Fix the login redirect loop");
    });

    it("shows an ASCII spinner, not the pulsing dot", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} activitySummary="Fix it" />);
        expect(container.querySelector(".agent-working-row .agent-spinner-dot")).toBeNull();
        expect(container.querySelector(".agent-working-row .agent-spinner-ascii")?.textContent).toMatch(/^[|/\*-]$/);
    });
});

/**
 * SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4: the row times and
 * counts the turn the user sees, over its CLI passes, from srv's ledger.
 */
describe("AgentWorkingRow across a turn's passes", () => {
    beforeEach(() => {
        vi.useFakeTimers(FAKE_CLOCK);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    const ledger = (over: Partial<TurnLedger> = {}): TurnLedger => ({
        turnId: 7,
        seq: 1,
        origin: "user",
        trigger: { kind: "user", from: null, external: false },
        absorbed: [],
        startedAtMs: Date.now() - 125_000,
        passes: 1,
        active: true,
        inputs: 0,
        countedPasses: 0,
        outputTokens: 0,
        costUsd: 0,
        steps: 0,
        durationApiMs: 0,
        lastPassEndedAtMs: null,
        settleUntilMs: null,
        endedAtMs: null,
        end: null,
        ...over,
    });

    it("runs the clock from the turn's start, not from this pass", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} turnLedger={ledger({ passes: 3 })} />);

        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^2m \d+s$/);
    });

    it("adds the running pass's output to the passes srv has counted", () => {
        const { container } = render(() => (
            <AgentWorkingRow
                loading={true}
                turnLedger={ledger({ passes: 2, countedPasses: 1, outputTokens: 2_000 })}
                turnTokens={{ input: 9_000, output: 300, ledgerTurnId: 7, ledgerPass: 2 }}
            />
        ));

        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^↓ 2\.3k tokens {2}·  2m \d+s$/);
    });

    it("between two passes stays the live row, dimmed and with no arrow, not a Worked line", () => {
        const now = Date.now();
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                activitySummary="Fix the login redirect loop"
                // The pass that just ended: its own stats, which the old row showed as "Worked".
                sessionStats={{ duration_ms: 5_000, output_tokens: 100, num_turns: 1 }}
                turnLedger={ledger({
                    active: false,
                    countedPasses: 1,
                    outputTokens: 2_000,
                    lastPassEndedAtMs: now,
                    settleUntilMs: now + 2_000,
                })}
            />
        ));
        finishTyping(container);
        const row = container.querySelector(".agent-working-row--loading");
        expect(row?.classList.contains("is-settling")).toBe(true);
        expect(container.querySelector(".agent-working-row--worked")).toBeNull();
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Fix the login redirect loop");
        expect(container.querySelector(".agent-working-row-right")?.textContent).toMatch(/^2\.0k tokens {2}·  2m \d+s$/);
    });

    it("reports the whole turn when it ends: wall time, every pass's output, cost, steps and passes", () => {
        const end = Date.now() - 1_000;
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                sessionStats={{ duration_ms: 5_000, output_tokens: 100, num_turns: 1, cost_usd: 0.01 }}
                turnLedger={ledger({
                    startedAtMs: end - 192_000,
                    active: false,
                    passes: 2,
                    countedPasses: 2,
                    outputTokens: 4_200,
                    costUsd: 0.41,
                    steps: 9,
                    lastPassEndedAtMs: end,
                    endedAtMs: end,
                    end: "completed",
                })}
            />
        ));

        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("✓ Worked  ·  3m 12s  ·  4.2k tokens");
        expect(container.querySelector(".agent-working-row-secondary")?.textContent).toBe("$0.410  ·  9 steps  ·  2 passes");
    });

    it("shows no cost for a turn whose provider reports none (Codex), not $0.000", () => {
        const end = Date.now() - 1_000;
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                sessionStats={{ duration_ms: 5_000, output_tokens: 100, num_turns: 1 }}
                turnLedger={ledger({
                    startedAtMs: end - 30_000,
                    active: false,
                    countedPasses: 1,
                    outputTokens: 1_500,
                    steps: 3,
                    lastPassEndedAtMs: end,
                    endedAtMs: end,
                    end: "completed",
                })}
            />
        ));

        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("✓ Worked  ·  30s  ·  1.5k tokens");
        expect(container.querySelector(".agent-working-row-secondary")?.textContent).toBe("3 steps");
    });

    it("opens an external turn by naming what started it, then goes back to its usual text", () => {
        const { container } = render(() => (
            <AgentWorkingRow
                loading={true}
                activitySummary="Fix the login redirect loop"
                turnLedger={ledger({ startedAtMs: Date.now() - 500, trigger: { kind: "agent", from: "AgentX", external: true } })}
            />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-primary")?.textContent).toBe("↳ jekt from AgentX");
        // Nothing after it: the goal is in the Swarm view.
        expect(container.querySelector(".agent-working-row-detail")).toBeNull();

        const later = render(() => (
            <AgentWorkingRow
                loading={true}
                activitySummary="Fix the login redirect loop"
                turnLedger={ledger({ startedAtMs: Date.now() - 10_000, trigger: { kind: "agent", from: "AgentX", external: true } })}
            />
        ));
        finishTyping(later.container);
        expect(later.container.querySelector(".agent-working-row-left")?.textContent).toBe("Fix the login redirect loop");
    });

    it("a turn the user started opens with its usual text", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="Fix it" turnLedger={ledger({ startedAtMs: Date.now() - 500 })} />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Fix it");
    });

    it("names an external turn on its Worked line, with what joined it", () => {
        const end = Date.now() - 1_000;
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                sessionStats={{ duration_ms: 5_000, output_tokens: 100, num_turns: 1 }}
                turnLedger={ledger({
                    trigger: { kind: "service", from: "github-consumer", external: true },
                    absorbed: [{ kind: "user", from: null, external: false }],
                    inputs: 1,
                    startedAtMs: end - 42_000,
                    active: false,
                    countedPasses: 2,
                    passes: 2,
                    outputTokens: 900,
                    costUsd: 0.05,
                    steps: 4,
                    lastPassEndedAtMs: end,
                    endedAtMs: end,
                    end: "completed",
                })}
            />
        ));
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe(
            "✓ Worked on github-consumer's notice  ·  42s  ·  900 tokens",
        );
        expect(container.querySelector(".agent-working-row-secondary")?.textContent).toBe(
            "$0.050  ·  4 steps  ·  2 passes  ·  +1 your message",
        );
    });

    it("reports the turn from the pane's own carry when srv counted no passes (ACP, App Server, subprocess)", () => {
        const end = Date.now() - 1_000;
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                sessionStats={{ duration_ms: 5_000, output_tokens: 40, num_turns: 1, cost_usd: 0.01 }}
                turnLedger={ledger({ startedAtMs: end - 30_000, active: false, passes: 2, lastPassEndedAtMs: end, endedAtMs: end, end: "completed" })}
                turnCarry={{ turnId: 7, passes: 2, outputTokens: 640, costUsd: 0.09, steps: 5 }}
            />
        ));
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("✓ Worked  ·  30s  ·  640 tokens");
        expect(container.querySelector(".agent-working-row-secondary")?.textContent).toBe("$0.090  ·  5 steps  ·  2 passes");
    });

    it("calls result.num_turns steps, which is what it counts", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={false} sessionStats={{ duration_ms: 42_000, num_turns: 3, cost_usd: 0.2 }} />
        ));

        expect(container.querySelector(".agent-working-row-secondary")?.textContent).toBe("$0.200  ·  3 steps");
    });
});

/**
 * SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6: the left zone is the
 * presenter's line (status/present-status.ts, tested there in depth).
 */
describe("AgentWorkingRow live status", () => {
    beforeEach(() => {
        vi.useFakeTimers(FAKE_CLOCK);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    const busy = (startedAt: number) => ({
        phase: null,
        phaseSince: 0,
        tools: [{ id: "t1", activity: { family: "bash" as const, label: "Running the srv test suite" }, startedAt }],
        plan: null,
        planAt: 0,
    });

    it("names what is running once it has run a moment, over the goal", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="Fix the login redirect loop" activity={busy(Date.now() - 3_000)} />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-primary")?.textContent).toBe("Running the srv test suite");
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Running the srv test suite");
    });

    it("keeps the goal while a call is too young to mention", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={true} activitySummary="Fix the login redirect loop" activity={busy(Date.now() - 200)} />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Fix the login redirect loop");
    });

    it("a question or approval for the user outranks everything", () => {
        const { container } = render(() => (
            <AgentWorkingRow
                loading={true}
                stopping={false}
                activity={busy(Date.now() - 3_000)}
                needsYou="Waiting for your approval: Running git push"
            />
        ));
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Waiting for your approval: Running git push");
    });

    it("a status like Stopping still outranks what is running", () => {
        const { container } = render(() => <AgentWorkingRow loading={true} stopping={true} activity={busy(Date.now() - 3_000)} />);
        finishTyping(container);
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Stopping…");
    });
});

// Tier 4 (SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §5):
// an ESTIMATED progress bar from earlier compactions' durations. Claude Code
// reports no real progress, so it is labeled as an estimate and never "done".
describe("AgentWorkingRow estimated compaction progress", () => {
    const seed = (durationMs: number, preTokens = 50000, uuid = `s-${durationMs}`) =>
        recordCompactionSample({ uuid, preTokens, durationMs });
    const startedAgo = (ms: number) => ({ trigger: "auto" as const, startedAt: Date.now() - ms });
    const right = (c: HTMLElement) => c.querySelector(".agent-working-row-right")?.textContent ?? "";
    const bar = (c: HTMLElement) => c.querySelector<HTMLElement>(".agent-working-row-progress");
    const fill = (c: HTMLElement) => c.querySelector<HTMLElement>(".agent-working-row-progress-fill");

    // The bar width is elapsed / estimate: with a running clock, the few ms
    // between `startedAgo` and the render made it 40.0033% instead of 40% on
    // a slow CI runner. Only `Date` is pinned; the row's interval timers stay real.
    beforeEach(() => {
        localStorage.clear();
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(new Date("2026-10-09T12:00:00Z"));
    });
    afterEach(() => {
        vi.useRealTimers();
        localStorage.clear();
    });

    it("shows only the elapsed counter, with no bar, when there is no history (R3)", () => {
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={startedAgo(12_000)} compactionContextTokens={50000} />
        ));
        expect(right(container)).toMatch(/^\d+s$/);
        expect(bar(container)).toBeNull();
    });

    it("shows elapsed / ~typical and an estimate bar once there is history", () => {
        seed(30_000);
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={startedAgo(12_000)} compactionContextTokens={50000} />
        ));
        expect(right(container)).toBe("12s / ~30s");
        expect(bar(container)?.getAttribute("role")).toBe("progressbar");
        expect(bar(container)?.getAttribute("aria-valuenow")).toBe("40");
        expect(bar(container)?.getAttribute("aria-valuetext")).toMatch(/estimated/);
        expect(fill(container)?.style.width).toBe("40%");
        expect(container.querySelector(".agent-working-row-right")?.getAttribute("title")).toMatch(/Estimate/);
    });

    it("on overshoot says 'longer than usual', goes indeterminate, and never reaches 100% (R2)", () => {
        seed(30_000);
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={startedAgo(45_000)} compactionContextTokens={50000} />
        ));
        expect(right(container)).toBe("45s · longer than usual");
        expect(bar(container)?.classList.contains("is-over")).toBe(true);
        expect(bar(container)?.hasAttribute("aria-valuenow")).toBe(false);
        expect(fill(container)?.style.width).toBe("95%");
    });

    it("keeps the same estimate for the whole compaction, whatever else changes (R4)", () => {
        seed(30_000);
        const [tokens, setTokens] = createSignal(50000);
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={startedAgo(12_000)} compactionContextTokens={tokens()} />
        ));
        expect(right(container)).toBe("12s / ~30s");
        seed(200_000, 50000, "late"); // a new sample lands mid-run
        setTokens(500_000); // and the context size changes
        expect(right(container)).toBe("12s / ~30s");
    });

    it("estimates afresh for the next compaction", () => {
        seed(30_000);
        const [compacting, setCompacting] = createSignal<ReturnType<typeof startedAgo> | null>(startedAgo(5_000));
        const { container } = render(() => (
            <AgentWorkingRow loading={false} compacting={compacting()} compactionContextTokens={50000} />
        ));
        expect(right(container)).toBe("5s / ~30s");
        seed(90_000, 50000, "slow"); // history is now [30s, 90s] -> median 60s
        // A different compaction has a different start time; keyed on it, the
        // estimate is computed afresh.
        setCompacting({ trigger: "auto", startedAt: Date.now() - 5_000 - 50 });
        expect(right(container)).toBe("5s / ~1m 0s");
    });

    it("shows no bar while reconnecting, which takes the row over", () => {
        seed(30_000);
        const { container } = render(() => (
            <AgentWorkingRow
                loading={false}
                compacting={startedAgo(12_000)}
                reconnecting={{ startedAt: Date.now() }}
                compactionContextTokens={50000}
            />
        ));
        expect(bar(container)).toBeNull();
        expect(container.querySelector(".agent-working-row-left")?.textContent).toBe("Reconnecting…");
    });
});

// SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3 — right
// after a launch (one-shot request) the composer takes focus itself, with
// retries; every other mount defers to focusManager.claimFocusOnMount
// (SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §3a).
describe("AgentFooter keyboard focus", () => {
    function makeFocusViewModel(blockId: string): AgentViewModel {
        return {
            blockId,
            blockAtom: () => ({ meta: {} }) as any,
            voiceTargetRef: { current: null },
            focusTargetRef: { current: null },
            giveFocus: () => false,
        } as unknown as AgentViewModel;
    }

    function spyClaim() {
        return vi.spyOn(focusManager, "claimFocusOnMount").mockImplementation(() => {});
    }

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("takes focus on mount when the launch path requested it, without the generic claim", () => {
        const claim = spyClaim();
        const vm = makeFocusViewModel("focus-block-launch");
        requestComposerFocus(vm.blockId);
        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(document.activeElement).toBe(getComposer());
        expect(claim).not.toHaveBeenCalled();
    });

    it("defers an ordinary mount (tab switch back, layout restore) to claimFocusOnMount", () => {
        const claim = spyClaim();
        const vm = makeFocusViewModel("focus-block-plain");
        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(claim).toHaveBeenCalledTimes(1);
        expect(claim.mock.calls[0][0]).toBe(vm.blockId);
        expect(document.activeElement).not.toBe(getComposer());
    });

    it("honors the request only once — a later remount of the same block goes through the generic claim", () => {
        const claim = spyClaim();
        const vm = makeFocusViewModel("focus-block-remount");
        requestComposerFocus(vm.blockId);
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        unmount();
        (document.activeElement as HTMLElement | null)?.blur();
        render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(claim).toHaveBeenCalledTimes(1);
        expect(document.activeElement).not.toBe(getComposer());
    });

    it("registers the textarea as giveFocus()'s target and clears it on unmount", () => {
        spyClaim();
        const vm = makeFocusViewModel("focus-block-handle");
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={vm} />);
        expect(vm.focusTargetRef.current).toBe(getComposer());
        unmount();
        expect(vm.focusTargetRef.current).toBeNull();
    });
});
