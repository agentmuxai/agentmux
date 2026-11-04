// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer paths the other AgentFooter tests leave out: the slash dropdown and its keys, IME
 * composition, Ctrl+R history search, recalling a queued message, the agent's own sent history, the
 * RAF-debounced typing callback, and the markup the footer draws. A refactor of the footer must leave
 * the committed snapshots as they are; regenerate with `vitest -u` only for a change meant to look different.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    rememberSent: vi.fn(),
    saveAgentDraft: vi.fn(),
    loadOlder: vi.fn(),
    history: [] as string[],
    storedDraft: undefined as string | undefined,
}));
vi.mock("../agent-input-history", async (orig) => {
    const real = (await orig()) as Record<string, unknown>;
    return {
        ...real,
        agentHistory: () => hub.history,
        watchAgentHistory: () => () => {},
        loadOlderAgentHistory: (...a: unknown[]) => hub.loadOlder(...a),
        rememberSent: (...a: unknown[]) => hub.rememberSent(...a),
        saveAgentDraft: (...a: unknown[]) => hub.saveAgentDraft(...a),
        agentDraft: async () => hub.storedDraft,
    };
});
vi.mock("./InputHistorySearch", () => ({
    InputHistorySearch: (p: { agentId: string; onDone: (text?: string) => void }) => (
        <div data-testid="history-search" data-agent={p.agentId}>
            <button data-testid="pick" onClick={() => p.onDone("picked entry")} />
            <button data-testid="cancel" onClick={() => p.onDone(undefined)} />
        </div>
    ),
}));
vi.mock("@/app/element/MicButton", () => ({ MicButton: (p: { blockId: string }) => <button data-mic={p.blockId} /> }));

import { AgentFooter } from "./AgentFooter";
import type { AgentViewModel } from "../agent-model";
import type { SlashCommand } from "../commands/types";
import { resetReloadPersistenceForTests } from "../reload-persistence";

const COMMANDS = [{ name: "help" }, { name: "history" }, { name: "hooks" }] as unknown as SlashCommand[];

let seq = 0;
function viewModel(meta: Record<string, unknown> = {}, voice = false): AgentViewModel {
    return {
        blockId: `keys-block-${seq++}`,
        blockAtom: () => ({ meta }) as any,
        voiceTargetRef: { current: null },
        focusTargetRef: { current: null },
        ...(voice ? { voiceHandle: () => ({ appendFinal: () => {}, setInterim: () => {} }) } : {}),
    } as unknown as AgentViewModel;
}

const composer = () => screen.getByPlaceholderText(/Send message to/) as HTMLTextAreaElement;
function key(el: Element, k: string, opts: KeyboardEventInit = {}) {
    const ev = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts });
    el.dispatchEvent(ev);
    return ev;
}
function typeInto(ta: HTMLTextAreaElement, text: string) {
    ta.value = text;
    fireEvent.input(ta);
}
const markup = (el: Element | null): string => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<") + "\n";
const snap = (name: string) => `./__snapshots__/AgentFooter.keys.${name}.html`;

beforeEach(() => {
    hub.rememberSent.mockClear();
    hub.saveAgentDraft.mockClear();
    hub.loadOlder.mockClear();
    hub.history = [];
    hub.storedDraft = undefined;
    resetReloadPersistenceForTests();
});
afterEach(() => cleanup());

describe("AgentFooter slash dropdown", () => {
    function setup() {
        const onSendMessage = vi.fn();
        const onTyping = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} onTyping={onTyping} getCompletions={(p) => COMMANDS.filter((c) => c.name.startsWith(p))} />);
        return { onSendMessage, onTyping, ta: composer() };
    }
    const selected = () => Array.from(document.querySelectorAll(".slash-autocomplete__row")).map((r) => r.classList.contains("slash-autocomplete__row--selected"));

    it("opens on a /prefix with no space, lists the matches, and closes once the text has a space or a newline", () => {
        const { ta } = setup();
        typeInto(ta, "/h");
        expect(document.querySelectorAll(".slash-autocomplete__row")).toHaveLength(3);
        typeInto(ta, "/hi");
        expect(document.querySelectorAll(".slash-autocomplete__row")).toHaveLength(1);
        typeInto(ta, "/history now");
        expect(document.querySelector(".slash-autocomplete")).toBeNull();
        typeInto(ta, "/h");
        typeInto(ta, "/h\nmore");
        expect(document.querySelector(".slash-autocomplete")).toBeNull();
    });

    it("arrows move the selection round the list and Escape closes it without touching the text", () => {
        const { ta } = setup();
        typeInto(ta, "/h");
        expect(selected()).toEqual([true, false, false]);
        expect(key(ta, "ArrowDown").defaultPrevented).toBe(true);
        expect(selected()).toEqual([false, true, false]);
        key(ta, "ArrowUp");
        key(ta, "ArrowUp");
        expect(selected()).toEqual([false, false, true]);
        key(ta, "ArrowDown");
        expect(selected()).toEqual([true, false, false]);
        expect(key(ta, "Escape").defaultPrevented).toBe(true);
        expect(document.querySelector(".slash-autocomplete")).toBeNull();
        expect(ta.value).toBe("/h");
    });

    it("Tab fills the selected command in for further editing; Enter fills it in and sends it", () => {
        const { ta, onSendMessage, onTyping } = setup();
        typeInto(ta, "/h");
        key(ta, "ArrowDown");
        onTyping.mockClear();
        key(ta, "Tab");
        expect(ta.value).toBe("/history ");
        expect(document.querySelector(".slash-autocomplete")).toBeNull();
        expect(onTyping).toHaveBeenCalledTimes(1);
        expect(onSendMessage).not.toHaveBeenCalled();

        typeInto(ta, "/ho");
        key(ta, "Enter");
        expect(onSendMessage).toHaveBeenCalledWith("/hooks ");
        expect(ta.value).toBe("");
    });

    it("a mouse pick on a row fills it in like Tab", () => {
        const { ta } = setup();
        typeInto(ta, "/h");
        const rows = document.querySelectorAll(".slash-autocomplete__row");
        fireEvent.mouseEnter(rows[2]);
        expect(selected()).toEqual([false, false, true]);
        fireEvent.mouseDown(rows[2]);
        expect(ta.value).toBe("/hooks ");
    });

    it("marks a !command and unmarks it when the text changes", () => {
        const { ta } = setup();
        typeInto(ta, "!ls");
        expect(ta.classList.contains("bang-command")).toBe(true);
        typeInto(ta, "ls");
        expect(ta.classList.contains("bang-command")).toBe(false);
    });
});

describe("AgentFooter IME composition", () => {
    it("holds off the dropdown and ignores Enter while composing, and catches up when composition ends", () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} getCompletions={() => COMMANDS} />);
        const ta = composer();
        fireEvent.compositionStart(ta);
        typeInto(ta, "/he");
        expect(document.querySelector(".slash-autocomplete")).toBeNull();
        expect(key(ta, "Enter", { isComposing: true }).defaultPrevented).toBe(false);
        expect(key(ta, "Enter", { keyCode: 229 }).defaultPrevented).toBe(false);
        expect(onSendMessage).not.toHaveBeenCalled();
        fireEvent.compositionEnd(ta);
        expect(document.querySelector(".slash-autocomplete")).not.toBeNull();
    });
});

describe("AgentFooter typing callback", () => {
    it("tells the pane once per animation frame however many keystrokes came, and not at all without a callback", () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        try {
            const onTyping = vi.fn();
            render(() => <AgentFooter agentName="Test" onTyping={onTyping} />);
            const ta = composer();
            typeInto(ta, "a");
            typeInto(ta, "ab");
            typeInto(ta, "abc");
            expect(frames).toHaveLength(1);
            expect(onTyping).not.toHaveBeenCalled();
            frames.shift()!(0);
            expect(onTyping).toHaveBeenCalledTimes(1);
            typeInto(ta, "abcd");
            expect(frames).toHaveLength(1);
        } finally {
            vi.unstubAllGlobals();
        }
    });
});

describe("AgentFooter with an agent of its own history", () => {
    it("Ctrl+R opens the history search, and a pick lands in the composer", () => {
        render(() => <AgentFooter agentName="Test" viewModel={viewModel({ agentId: "uid-1" })} />);
        const ta = composer();
        expect(screen.queryByTestId("history-search")).toBeNull();
        expect(key(ta, "r", { ctrlKey: true }).defaultPrevented).toBe(true);
        expect(screen.getByTestId("history-search").getAttribute("data-agent")).toBe("uid-1");
        fireEvent.click(screen.getByTestId("pick"));
        expect(screen.queryByTestId("history-search")).toBeNull();
        expect(ta.value).toBe("picked entry");
        expect(hub.saveAgentDraft).toHaveBeenLastCalledWith("uid-1", "picked entry");
    });

    it("cancelling the search keeps the text and returns focus to the composer", () => {
        render(() => <AgentFooter agentName="Test" viewModel={viewModel({ agentId: "uid-1" })} />);
        const ta = composer();
        typeInto(ta, "keep me");
        key(ta, "r", { ctrlKey: true });
        fireEvent.click(screen.getByTestId("cancel"));
        expect(ta.value).toBe("keep me");
        expect(document.activeElement).toBe(ta);
    });

    it("Ctrl+R does nothing for a pane without an agent of its own, and with Shift or Alt held", () => {
        const { unmount } = render(() => <AgentFooter agentName="Test" viewModel={viewModel({})} />);
        expect(key(composer(), "r", { ctrlKey: true }).defaultPrevented).toBe(false);
        expect(screen.queryByTestId("history-search")).toBeNull();
        unmount();
        render(() => <AgentFooter agentName="Test" viewModel={viewModel({ agentId: "uid-1" })} />);
        expect(key(composer(), "r", { ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(false);
        expect(key(composer(), "r", { ctrlKey: true, altKey: true }).defaultPrevented).toBe(false);
    });

    it("records a send in the agent's history, saves each edit as its draft, and ArrowUp near the oldest entry reads the page before", () => {
        hub.history = ["old one", "old two"];
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" viewModel={viewModel({ agentId: "uid-1" })} onSendMessage={onSendMessage} />);
        const ta = composer();
        typeInto(ta, "hello");
        expect(hub.saveAgentDraft).toHaveBeenLastCalledWith("uid-1", "hello");
        key(ta, "Enter");
        expect(onSendMessage).toHaveBeenCalledWith("hello");
        expect(hub.rememberSent).toHaveBeenCalledWith("uid-1", "hello");
        expect(hub.saveAgentDraft).toHaveBeenLastCalledWith("uid-1", "");
        key(ta, "ArrowUp");
        expect(ta.value).toBe("old two");
        expect(hub.loadOlder).toHaveBeenCalledWith("uid-1");
    });

    it("restores the agent's own draft into an empty composer, but not over text already typed", async () => {
        hub.storedDraft = "draft from before";
        render(() => <AgentFooter agentName="Test" viewModel={viewModel({ agentId: "uid-1" })} />);
        await Promise.resolve();
        await Promise.resolve();
        expect(composer().value).toBe("draft from before");
    });
});

describe("AgentFooter recalling a queued message", () => {
    it("ArrowUp on an empty composer un-queues the latest held message before any history", () => {
        const onRecallLatestQueued = vi.fn().mockReturnValue({ text: "queued text" });
        render(() => <AgentFooter agentName="Test" onRecallLatestQueued={onRecallLatestQueued} />);
        const ta = composer();
        expect(key(ta, "ArrowUp").defaultPrevented).toBe(true);
        expect(onRecallLatestQueued).toHaveBeenCalledTimes(1);
        expect(ta.value).toBe("queued text");
    });

    it("falls through to history when nothing is queued, and does not ask while the composer has text", () => {
        const onRecallLatestQueued = vi.fn().mockReturnValue(null);
        render(() => <AgentFooter agentName="Test" onRecallLatestQueued={onRecallLatestQueued} />);
        const ta = composer();
        key(ta, "ArrowUp");
        expect(onRecallLatestQueued).toHaveBeenCalledTimes(1);
        expect(ta.value).toBe("");
        typeInto(ta, "text");
        ta.setSelectionRange(0, 0);
        key(ta, "ArrowUp");
        expect(onRecallLatestQueued).toHaveBeenCalledTimes(1);
    });
});

describe("AgentFooter keys that are not the composer's", () => {
    it("Escape clears text, and on an empty composer stops the agent", () => {
        const onStopAgent = vi.fn();
        render(() => <AgentFooter agentName="Test" onStopAgent={onStopAgent} />);
        const ta = composer();
        typeInto(ta, "some words");
        expect(key(ta, "Escape").defaultPrevented).toBe(true);
        expect(ta.value).toBe("");
        expect(onStopAgent).not.toHaveBeenCalled();
        key(ta, "Escape");
        expect(onStopAgent).toHaveBeenCalledTimes(1);
    });

    it("Shift+Enter is left to the textarea, and a blank message is not sent", () => {
        const onSendMessage = vi.fn();
        render(() => <AgentFooter agentName="Test" onSendMessage={onSendMessage} />);
        const ta = composer();
        expect(key(ta, "Enter", { shiftKey: true }).defaultPrevented).toBe(false);
        typeInto(ta, "   ");
        key(ta, "Enter");
        expect(onSendMessage).not.toHaveBeenCalled();
    });
});

describe("AgentFooter markup", () => {
    it("the plain footer", async () => {
        const { container } = render(() => <AgentFooter agentName="Korp" />);
        await expect(markup(container.querySelector(".agent-footer"))).toMatchFileSnapshot(snap("plain"));
    });

    it("with a voice-enabled pane, a !command in the box, and the slash dropdown open", async () => {
        const { container } = render(() => (
            <AgentFooter agentName="Korp" viewModel={viewModel({}, true)} getCompletions={() => COMMANDS} />
        ));
        typeInto(composer(), "/h");
        await expect(markup(container.querySelector(".agent-footer"))).toMatchFileSnapshot(snap("voice-dropdown"));
        typeInto(composer(), "!ls");
        await expect(markup(container.querySelector(".agent-footer"))).toMatchFileSnapshot(snap("voice-bang"));
    });
});
