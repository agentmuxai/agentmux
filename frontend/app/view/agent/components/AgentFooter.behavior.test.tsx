// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter (the agent pane's composer), pinned as a whole ahead of splitting it into a state object and views:
 * each scenario mounts the real footer with stand-ins for the composer's actions and key handling, its paste, voice,
 * focus and placeholder hooks, the attachment tray, the slash dropdown, the history search, the mic button, the sent
 * history and the context menu, drives the textarea (keys, input, paste, composition, right-click) and the stand-ins,
 * and logs the footer as an indented tree and, in order, every call it makes, each with the composer context it was
 * handed (the draft's block, the history, the Esc-undo and composing flags, the dropdown's prefix and index).
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (AgentFooter.*.test.tsx assert the composer's behaviour against the real actions; this pins the wiring and the markup.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ watcher: null as any }));

/** What the composer's functions are handed, as one line. */
const ctxLine = (c: any): string =>
    JSON.stringify({
        block: c.draftBlockId,
        agent: c.historyAgent,
        draft: c.attachmentDraft?.id,
        sent: c.sentHistory,
        hist: c.hist,
        esc: c.escClearedDraft,
        composing: c.composing,
        scroll: c.typingScrollPending,
        prefix: c.autocompletePrefix(),
        index: c.autocompleteIndex(),
        completions: c.completions().map((x: any) => x.name),
        textarea: c.textarea()?.tagName,
    });

vi.mock("@/app/store/contextmenu", () => ({
    showTextInputContextMenu: (_e: unknown, items: unknown, extra: any) => {
        log(`showTextInputContextMenu ${JSON.stringify(items)} ${extra ? Object.keys(extra).join(",") : "-"}`);
        extra?.onPasteAttachments?.();
    },
}));
vi.mock("@/app/element/MicButton", () => ({
    MicButton: (p: { blockId: string; handle: unknown; paneTitle: string }) => (
        <span data-stand="MicButton" data-block-id={p.blockId} data-handle={JSON.stringify(p.handle)} data-title={p.paneTitle} />
    ),
}));
vi.mock("@/app/keybindings", () => ({ shortcutFor: (id: string) => `<${id}>` }));
vi.mock("./SlashAutocomplete", () => ({
    SlashAutocomplete: (p: {
        completions: { name: string }[];
        selectedIndex: number;
        onHover: (i: number) => void;
        onSelect: (cmd: { name: string }) => void;
    }) => (
        <div data-stand="SlashAutocomplete" data-selected={p.selectedIndex} data-names={p.completions.map((x) => x.name).join(",")}>
            <span data-stand="hover" onClick={() => (log("SlashAutocomplete onHover 1"), p.onHover(1))} />
            <span data-stand="select" onClick={() => (log("SlashAutocomplete onSelect"), p.onSelect(p.completions[0]))} />
        </div>
    ),
}));
vi.mock("./InputHistorySearch", () => ({
    InputHistorySearch: (p: { agentId: string; onDone: (text?: string) => void }) => (
        <div data-stand="InputHistorySearch" data-agent={p.agentId}>
            <span data-stand="pick" onClick={() => (log("InputHistorySearch onDone text"), p.onDone("from history"))} />
            <span data-stand="cancel" onClick={() => (log("InputHistorySearch onDone"), p.onDone(undefined))} />
        </div>
    ),
}));
vi.mock("../attachments/AttachmentTray", () => ({
    AttachmentTray: (p: { draft: { id: string }; focusComposer: () => void; registerFocusLast: (fn: () => void) => void }) => {
        log(`AttachmentTray ${p.draft.id}`);
        p.registerFocusLast(() => log("focusLast"));
        return (
            <div data-stand="AttachmentTray" data-draft={p.draft.id}>
                <span data-stand="focus" onClick={() => (log("AttachmentTray focusComposer"), p.focusComposer())} />
            </div>
        );
    },
}));
vi.mock("../attachments/attachment-draft", () => ({
    getAttachmentDraft: (blockId: string) => (log(`getAttachmentDraft ${blockId}`), { id: `draft-${blockId}` }),
}));
vi.mock("../composer-restore", () => ({
    useComposerRestore: (blockId: string | undefined, textarea: () => HTMLElement | undefined, write: (text: string) => void) => {
        log(`useComposerRestore ${blockId} ${textarea()?.tagName}`);
        write("restored text");
    },
}));
vi.mock("../reload-persistence", () => ({
    getSentHistory: (blockId: string) => (log(`getSentHistory ${blockId}`), ["block one", "block two"]),
}));
vi.mock("../agent-input-history", () => ({
    historyAgentId: (meta: any) => (log(`historyAgentId ${JSON.stringify(meta)}`), meta?.agent ?? null),
    agentHistory: (agent: string) => (log(`agentHistory ${agent}`), ["agent one", "agent two", "agent three"]),
    watchAgentHistory: (agent: string, cb: (where: string, n: number, at: number) => void) => {
        log(`watchAgentHistory ${agent}`);
        h.watcher = cb;
        return () => log("unwatch");
    },
    movedCursor: (pos: number, length: number, where: string, n: number, at: number) => {
        log(`movedCursor ${pos} ${length} ${where} ${n} ${at}`);
        return pos + n;
    },
}));
vi.mock("../hooks/useComposerPaste", () => ({
    useComposerPaste: (opts: { blockId?: string; draft?: { id: string }; textarea: () => HTMLElement | undefined }) => {
        log(`useComposerPaste ${opts.blockId} ${opts.draft?.id} ${opts.textarea()?.tagName}`);
        return {
            handlePaste: (e: Event) => log(`handlePaste ${e.type}`),
            pasteAttachmentsFromMenu: () => log("pasteAttachmentsFromMenu"),
        };
    },
}));
vi.mock("../hooks/useComposerHandles", () => ({
    useComposerVoiceHandle: (vm: any, textarea: () => HTMLElement | undefined) =>
        log(`useComposerVoiceHandle ${vm?.blockId} ${textarea()?.tagName}`),
    useComposerFocusHandle: (vm: any, textarea: () => HTMLElement | undefined) =>
        log(`useComposerFocusHandle ${vm?.blockId} ${textarea()?.tagName}`),
}));
vi.mock("../hooks/useComposerPlaceholder", () => ({
    useComposerPlaceholder: (opts: { viewModel: any; agentName: () => string }) => {
        log(`useComposerPlaceholder ${opts.viewModel?.blockId} ${opts.agentName()}`);
        return {
            visibleSuggestion: () => "a suggestion",
            placeholder: () => `Send message to ${opts.agentName()}`,
            maskCurrentSuggestion: () => log("maskCurrentSuggestion"),
        };
    },
}));
vi.mock("./composer-actions", () => ({
    acceptCompletion: (c: any, cmd: { name: string }) => log(`acceptCompletion ${cmd.name} ${ctxLine(c)}`),
    composerUndoItems: (c: any) => (log(`composerUndoItems ${ctxLine(c)}`), ["undo"]),
    handleInput: (c: any) => {
        log(`handleInput ${ctxLine(c)}`);
        c.setAutocompletePrefix("he");
        c.setAutocompleteIndex(2);
        c.setIsBangCmd(true);
        c.typingScrollPending = true;
    },
    restoreDraftOnMount: (c: any) => log(`restoreDraftOnMount ${ctxLine(c)}`),
    setComposerValue: (c: any, text: string) => log(`setComposerValue ${JSON.stringify(text)} ${ctxLine(c)}`),
    updateAutocomplete: (c: any) => log(`updateAutocomplete ${ctxLine(c)}`),
}));
vi.mock("./composer-keys", () => ({
    handleKeyDown: (c: any, e: KeyboardEvent) => {
        log(`handleKeyDown ${e.key} ${ctxLine(c)}`);
        if (e.key === "r") c.setHistorySearch(true);
        if (e.key === "h") c.showSendHint(`hint ${e.code || "x"}`);
        if (e.key === "f") c.focusLastTile?.();
        if (e.key === "m") c.maskCurrentSuggestion();
        if (e.key === "c") c.setAutocompletePrefix(null);
    },
}));

import { AgentFooter } from "./AgentFooter";

const SNAP = (name: string) => `./__snapshots__/AgentFooter.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|placeholder|rows|spellcheck)$/;

/** The footer as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let unmount: () => void;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const area = () => host.querySelector("textarea")!;
const click = (selector: string) => fireEvent.click(host.querySelector(selector)!);
const key = (k: string, code = "") => fireEvent.keyDown(area(), { key: k, code });

const callbacks = {
    onSendMessage: () => {},
    onTyping: () => {},
    getCompletions: (prefix: string) => (log(`getCompletions ${JSON.stringify(prefix)}`), [{ name: `${prefix}-one` }, { name: `${prefix}-two` }]),
};
const viewModel = (extra: Record<string, unknown> = {}) =>
    ({
        blockId: "blk-1",
        blockAtom: () => ({ meta: { agent: "agent-7" } }),
        voiceHandle: () => ({ voice: true }),
        ...extra,
    }) as any;

function mount(props: Record<string, unknown> = {}) {
    const view = render(() => <AgentFooter agentName="Claude" {...(props as any)} />, {
        container: document.body.appendChild(document.createElement("div")),
    });
    host = view.container;
    unmount = view.unmount;
    return view;
}

beforeEach(() => {
    T.length = 0;
    h.watcher = null;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

describe("AgentFooter, pinned", () => {
    it("for a caller with no view model: wires the hooks in order with nothing to persist, draws only the textarea, and hands the parent its isEmpty check", () => {
        let isEmpty: (() => boolean) | undefined;
        mount({ isComposerEmptyRef: (fn: () => boolean) => (log("isComposerEmptyRef"), (isEmpty = fn)) });
        look("no view model");
        log(`empty: ${isEmpty!()}`);
        area().value = "typed";
        log(`empty after typing: ${isEmpty!()}`);
        return snapshot("bare");
    });

    it("for a pane with a view model and a defined agent: keeps the agent's history (watching it, moving the cursor as entries arrive), draws the attachment tray and the mic button in the pane's shortcut, and cleans up on unmount", () => {
        mount({ viewModel: viewModel(), ...callbacks });
        look("view model");
        h.watcher("older", 2, 0);
        h.watcher("newer", 1, 3);
        click("[data-stand='AttachmentTray'] [data-stand='focus']");
        key("f");
        unmount();
        log("unmounted");
        return snapshot("pane");
    });

    it("keeps a pane's own sent history for a view model with no defined agent, and draws no mic button without a voice handle", () => {
        mount({
            viewModel: viewModel({ blockAtom: () => ({ meta: {} }), voiceHandle: undefined }),
            ...callbacks,
        });
        look("a block's own history");
        key("a");
        look("after one key");
        return snapshot("block-history");
    });

    it("hands the textarea's key, input, paste, composition and right-click events to the composer with its context, shows the bang class and the slash dropdown the input sets up, and passes a hover and a pick on", () => {
        mount({ viewModel: viewModel(), ...callbacks });
        key("a");
        fireEvent.input(area());
        look("after input");
        click("[data-stand='SlashAutocomplete'] [data-stand='hover']");
        look("hovered");
        click("[data-stand='SlashAutocomplete'] [data-stand='select']");
        fireEvent.paste(area());
        fireEvent.compositionStart(area());
        key("a");
        fireEvent.compositionEnd(area());
        key("a");
        fireEvent.contextMenu(area());
        key("m");
        key("c");
        look("dropdown closed by a key");
        return snapshot("events");
    });

    it("hides the dropdown without completions or without a completer, opens the history search from a key only for a defined agent, and writes what it returns (or nothing) into the composer", () => {
        mount({ viewModel: viewModel() });
        fireEvent.input(area());
        look("a prefix with no completer");
        unmount();
        document.body.innerHTML = "";
        mount({ viewModel: viewModel({ blockAtom: () => ({ meta: {} }) }), ...callbacks });
        key("r");
        look("history search for a block with no agent");
        unmount();
        document.body.innerHTML = "";
        mount({ viewModel: viewModel(), ...callbacks });
        key("r");
        look("history search open");
        click("[data-stand='InputHistorySearch'] [data-stand='pick']");
        look("search done with text");
        key("r");
        click("[data-stand='InputHistorySearch'] [data-stand='cancel']");
        look("search done without text");
        return snapshot("history-search");
    });

    it("shows a send hint for three seconds, restarts the three seconds with the next hint, and cancels the timer on unmount", () => {
        mount({ viewModel: viewModel(), ...callbacks });
        key("h", "first");
        look("first hint");
        vi.advanceTimersByTime(2000);
        key("h", "second");
        vi.advanceTimersByTime(2000);
        look("second hint, 4 s after the first");
        vi.advanceTimersByTime(1000);
        look("hint gone");
        key("h", "third");
        unmount();
        log(`timers pending after unmount: ${vi.getTimerCount()}`);
        return snapshot("send-hint");
    });
});
