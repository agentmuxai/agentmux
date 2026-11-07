// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it, vi } from "vitest";
import type { AttentionItem } from "./attention-list";
import {
    ATTENTION_TOAST_ID,
    buildToast,
    createAttentionToast,
    START_GRACE_MS,
    TOAST_MAX_LINKS,
} from "./attention-toast";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => null } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: () => {}, removeNotificationById: () => {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("./usenotification", () => ({ setNotificationAction: () => {} }));
vi.mock("./attention-list", () => ({ attentionItems: () => [] }));

const item = (blockId: string, over: Partial<AttentionItem> = {}): AttentionItem => ({
    blockId,
    name: `Agent-${blockId}`,
    color: "#3b82f6",
    what: "asks you a question",
    since: 0,
    local: true,
    ...over,
});

function harness(opts: { enabled?: boolean; inFront?: boolean; focused?: string | null } = {}) {
    let items: AttentionItem[] = [];
    let t = 0;
    const state = { enabled: opts.enabled ?? true, inFront: opts.inFront ?? true, focused: opts.focused ?? null };
    const pushed: NotificationType[] = [];
    const removed: string[] = [];
    const actions = new Map<string, () => void>();
    const revealed: string[] = [];
    const toast = createAttentionToast({
        items: () => items,
        enabled: () => state.enabled,
        inFront: () => state.inFront,
        focusedBlock: () => state.focused,
        push: (n) => pushed.push(n),
        remove: (id) => removed.push(id),
        setAction: (k, fn) => actions.set(k, fn),
        reveal: (b) => revealed.push(b),
        now: () => t,
    });
    return {
        state,
        pushed,
        removed,
        actions,
        revealed,
        set: (next: AttentionItem[]) => {
            items = next;
            toast.update();
        },
        wait: (ms: number) => void (t += ms),
        last: () => pushed[pushed.length - 1],
    };
}

// SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.2, §3.4, §3.5.
describe("the attention toast", () => {
    it("names one agent in its colour, says what it wants, and links to it", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1", { name: "Korp", what: "asks to approve a Bash call" })]);
        expect(h.last()).toMatchObject({
            id: ATTENTION_TOAST_ID,
            title: "Korp",
            titleColor: "#3b82f6",
            message: "asks to approve a Bash call",
            persistent: true,
            actions: [{ label: "Go to agent", actionKey: "attention:go:b1" }],
        });
        h.actions.get("attention:go:b1")!();
        expect(h.revealed).toEqual(["b1"]);
    });

    it("says another window holds it", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1", { local: false })]);
        expect(h.last().message).toBe("asks you a question (in another window)");
    });

    it("groups several into one toast, rewritten in place, with a link per agent and the rest counted", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.set([item("b1"), item("b2")]);
        expect(h.pushed.map((n) => n.id)).toEqual([ATTENTION_TOAST_ID, ATTENTION_TOAST_ID]);
        expect(h.last().title).toBe("2 agents need you");
        expect(h.last().actions!.map((a) => a.actionKey)).toEqual(["attention:go:b1", "attention:go:b2"]);
        const many = Array.from({ length: TOAST_MAX_LINKS + 2 }, (_, i) => item(`m${i}`));
        const toast = buildToast(
            many,
            () => {},
            () => {}
        );
        expect(toast.message).toMatch(/and 2 more$/);
        expect(toast.actions).toHaveLength(TOAST_MAX_LINKS);
    });

    it("does not toast for what was waiting when the window opened", () => {
        const h = harness();
        h.set([item("b1")]);
        h.wait(START_GRACE_MS - 1);
        h.set([item("b1"), item("b2")]);
        expect(h.pushed).toEqual([]);
        h.wait(1);
        h.set([item("b1"), item("b2")]);
        expect(h.pushed).toEqual([]);
        h.set([item("b1"), item("b2"), item("b3")]);
        expect(h.last().title).toBe("3 agents need you");
    });

    it("clears itself when the list empties", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.set([]);
        expect(h.removed).toEqual([ATTENTION_TOAST_ID]);
        h.set([]);
        expect(h.removed).toHaveLength(1);
    });

    it("drops an answered agent from the toast and keeps the rest", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1"), item("b2")]);
        h.set([item("b2")]);
        expect(h.last()).toMatchObject({ title: "Agent-b2", actions: [{ actionKey: "attention:go:b2" }] });
    });

    it("does not come back for the same requests after its X, but does for a new one", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.last().onClose!();
        h.set([item("b1")]);
        expect(h.pushed).toHaveLength(1);
        h.set([item("b1"), item("b2")]);
        expect(h.pushed).toHaveLength(2);
        expect(h.last()).toMatchObject({ title: "Agent-b2" });
    });

    it("shows nothing when no window is in front (the OS notification is the signal), or when it is turned off", () => {
        const away = harness({ inFront: false });
        away.wait(START_GRACE_MS);
        away.set([item("b1")]);
        expect(away.pushed).toEqual([]);
        const off = harness({ enabled: false });
        off.wait(START_GRACE_MS);
        off.set([item("b1")]);
        expect(off.pushed).toEqual([]);
    });

    it("goes when the setting is turned off while it shows, and stays when the window stops being in front", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.state.inFront = false;
        h.set([item("b1")]);
        expect(h.removed).toEqual([]);
        h.state.enabled = false;
        h.set([item("b1")]);
        expect(h.removed).toEqual([ATTENTION_TOAST_ID]);
        h.set([item("b1")]);
        expect(h.removed).toHaveLength(1);
    });

    it("leaves out the pane the person is looking at, which shows its own request", () => {
        const h = harness({ focused: "b1" });
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        expect(h.pushed).toEqual([]);
        h.set([item("b1"), item("b2")]);
        expect(h.last()).toMatchObject({ title: "Agent-b2" });
    });
});
