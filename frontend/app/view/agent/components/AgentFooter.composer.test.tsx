// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer's wiring to the rest of the pane, pinned before AgentFooter was split into hooks: the
 * voice handle the mic writes through, the placeholder in each of its three states, the focus handle,
 * and paste from the right-click menu. The keys, history, drafts and attachments are in
 * AgentFooter.test.tsx and AgentFooter.attachments.test.tsx.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    listening: false,
    target: null as string | null,
    container: false,
    attachmentsEnabled: undefined as boolean | undefined,
    copy: vi.fn(),
    menu: vi.fn(),
    claim: vi.fn(),
}));
vi.mock("@/app/hook/useVoiceInput", () => ({
    getVoiceSession: () => ({ isListening: () => hub.listening, currentTargetId: () => hub.target }),
}));
vi.mock("@/app/store/global", async (orig) => {
    const real = (await orig()) as { getSettingsKeyAtom: (key: string) => () => unknown };
    return { ...real, getSettingsKeyAtom: (key: string) => (key === "attachments:enabled" ? () => hub.attachmentsEnabled : real.getSettingsKeyAtom(key)) };
});
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: (...a: unknown[]) => hub.menu(...a) }));
vi.mock("@/app/drag/file-drop-actions", () => ({ copyIntoWorkdir: (...a: unknown[]) => hub.copy(...a) }));
vi.mock("../hooks/useAgentDropAttach", () => ({ isContainerPane: () => hub.container, spliceComposerTokens: () => {} }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { claimFocusOnMount: (...a: unknown[]) => hub.claim(...a), refocusNode: () => {} } }));

import { AgentFooter } from "./AgentFooter";
import type { AgentViewModel } from "../agent-model";
import { getAttachmentDraft } from "../attachments/attachment-draft";
import { META_SUGGESTION, META_SUGGESTION_GEN } from "@/app/store/meta-keys";

afterEach(() => cleanup());
beforeEach(() => {
    hub.listening = false;
    hub.target = null;
    hub.container = false;
    hub.attachmentsEnabled = undefined;
    hub.copy.mockReset();
    hub.menu.mockReset();
    hub.claim.mockReset();
});

let seq = 0;
function setup(meta: Record<string, unknown> = {}, extra: Partial<Parameters<typeof AgentFooter>[0]> = {}) {
    const blockId = `composer-block-${seq++}`;
    const vm = {
        blockId,
        blockAtom: () => ({ meta }) as never,
        voiceTargetRef: { current: null as unknown },
        focusTargetRef: { current: null as unknown },
        giveFocus: () => true,
    } as unknown as AgentViewModel & { voiceTargetRef: { current: { appendFinal(t: string): void; setInterim(t: string): void } | null }; focusTargetRef: { current: unknown } };
    const view = render(() => <AgentFooter agentName="Test" viewModel={vm} {...extra} />);
    const ta = view.container.querySelector("textarea") as HTMLTextAreaElement;
    return { vm, ta, blockId, ...view };
}

describe("the voice handle", () => {
    it("is registered while the composer is on screen and withdrawn when it goes", () => {
        const { vm, unmount } = setup();
        expect(vm.voiceTargetRef.current).not.toBeNull();
        unmount();
        expect(vm.voiceTargetRef.current).toBeNull();
    });

    it("appends a final transcript after the text with a space, and tells the composer it was edited", () => {
        const { vm, ta } = setup();
        const onInput = vi.fn();
        ta.addEventListener("input", onInput);
        ta.value = "hello ";
        vm.voiceTargetRef.current!.appendFinal("there");
        expect(ta.value).toBe("hello there ");
        vm.voiceTargetRef.current!.appendFinal("again");
        expect(ta.value).toBe("hello there again ");
        expect(onInput).toHaveBeenCalledTimes(2);
    });

    it("shows interim words after the base without committing them", () => {
        const { vm, ta } = setup();
        ta.value = "fix ";
        vm.voiceTargetRef.current!.setInterim("the log");
        expect(ta.value).toBe("fix the log");
        vm.voiceTargetRef.current!.setInterim("the login bug");
        expect(ta.value).toBe("fix the login bug");
        vm.voiceTargetRef.current!.appendFinal("the login bug");
        expect(ta.value).toBe("fix the login bug ");
    });

    it("keeps what the person typed between two voice events", () => {
        const { vm, ta } = setup();
        vm.voiceTargetRef.current!.appendFinal("one");
        expect(ta.value).toBe("one ");
        ta.value = "one two ";
        vm.voiceTargetRef.current!.appendFinal("three");
        expect(ta.value).toBe("one two three ");
    });
});

describe("the placeholder", () => {
    const placeholder = (ta: HTMLTextAreaElement) => ta.placeholder;

    it("names the agent", () => {
        expect(placeholder(setup().ta)).toBe("Send message to Test...");
    });

    it("says Speak to while this pane owns the voice session", () => {
        hub.listening = true;
        hub.target = `composer-block-${seq}`; // the id the next setup() gives its pane
        expect(placeholder(setup().ta)).toBe("Speak to Test...");
    });

    it("does not for a session on another pane, or one that is not listening", () => {
        hub.listening = true;
        hub.target = "someone-else";
        expect(placeholder(setup().ta)).toBe("Send message to Test...");
        hub.listening = false;
        hub.target = `composer-block-${seq}`;
        expect(placeholder(setup().ta)).toBe("Send message to Test...");
    });

    it("shows the next-prompt suggestion in place of both", () => {
        hub.listening = true;
        const { ta } = setup({ [META_SUGGESTION]: "Run the tests", [META_SUGGESTION_GEN]: 3 });
        expect(placeholder(ta)).toBe("Run the tests");
    });

    it("hides the suggestion that was current when a message was sent", () => {
        const { ta } = setup({ [META_SUGGESTION]: "Run the tests", [META_SUGGESTION_GEN]: 3 }, { onSendMessage: vi.fn() });
        ta.value = "go";
        fireEvent.keyDown(ta, { key: "Enter" });
        expect(placeholder(ta)).toBe("Send message to Test...");
    });
});

describe("the focus handle", () => {
    it("registers the textarea for giveFocus, claims focus on mount, and lets go when it goes", () => {
        const { vm, ta, unmount } = setup();
        expect(vm.focusTargetRef.current).toBe(ta);
        expect(hub.claim).toHaveBeenCalledTimes(1);
        expect(hub.claim.mock.calls[0][0]).toBe(vm.blockId);
        unmount();
        expect(vm.focusTargetRef.current).toBeNull();
    });
});

describe("paste from the right-click menu", () => {
    const openMenu = (ta: HTMLTextAreaElement) => {
        fireEvent.contextMenu(ta);
        const [, undoItems, options] = hub.menu.mock.calls.at(-1)!;
        return { undoItems, options } as { undoItems: Array<{ label: string; enabled: boolean }>; options?: { onPasteAttachments: (paths: string[]) => void } };
    };

    it("offers Undo, enabled only with text to undo", () => {
        const { ta } = setup();
        expect(openMenu(ta).undoItems.map((i) => [i.label, i.enabled])).toEqual([["Undo", false]]);
        ta.value = "draft";
        expect(openMenu(ta).undoItems[0].enabled).toBe(true);
    });

    it("hands the pasted paths to the attachment tray", () => {
        const { ta, blockId } = setup();
        const ingest = vi.spyOn(getAttachmentDraft(blockId), "ingestPaths").mockResolvedValue(undefined as never);
        openMenu(ta).options!.onPasteAttachments(["/tmp/a.png"]);
        expect(ingest).toHaveBeenCalledWith(["/tmp/a.png"]);
        expect(hub.copy).not.toHaveBeenCalled();
    });

    it("copies them into the working folder instead in a container pane or with attachments off", () => {
        hub.container = true;
        const { ta, blockId } = setup();
        const ingest = vi.spyOn(getAttachmentDraft(blockId), "ingestPaths").mockResolvedValue(undefined as never);
        openMenu(ta).options!.onPasteAttachments(["/tmp/a.png"]);
        expect(hub.copy).toHaveBeenCalledTimes(1);
        expect(hub.copy.mock.calls[0][0]).toBe(blockId);
        expect(hub.copy.mock.calls[0][1]).toEqual({ paths: ["/tmp/a.png"] });
        expect(hub.copy.mock.calls[0][2]).toMatchObject({ paneKind: "agent pane" });
        expect(ingest).not.toHaveBeenCalled();

        hub.container = false;
        hub.attachmentsEnabled = false;
        hub.copy.mockClear();
        openMenu(ta).options!.onPasteAttachments(["/tmp/b.png"]);
        expect(hub.copy.mock.calls[0][1]).toEqual({ paths: ["/tmp/b.png"] });
    });

    it("offers no paste option to a composer without a pane to attach to", () => {
        const view = render(() => <AgentFooter agentName="Test" />);
        fireEvent.contextMenu(view.container.querySelector("textarea")!);
        expect(hub.menu.mock.calls.at(-1)![2]).toBeUndefined();
    });
});
