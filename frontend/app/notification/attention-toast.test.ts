// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it, vi } from "vitest";
import type { AttentionItem } from "./attention-list";
import {
    ATTENTION_TOAST_ID,
    buildToast,
    createAttentionToast,
    noticesPaused,
    RETURN_MS,
    START_GRACE_MS,
    TOAST_LIFETIME_MS,
    TOAST_MAX_LINKS,
} from "./attention-toast";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => null } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: () => {}, removeNotificationById: () => {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("@/app/block/pane-color-scheme", () => ({ paneRoleColor: () => undefined }));
vi.mock("@/app/block/pane-identity", () => ({ isLightThemeActive: () => false }));
vi.mock("./usenotification", () => ({ setNotificationAction: () => {} }));
vi.mock("./attention-list", () => ({ attentionItems: () => [], attentionTint: (hex?: string) => hex }));
vi.mock("./attention-go", () => ({ goToAttention: () => {} }));

const item = (blockId: string, over: Partial<AttentionItem> = {}): AttentionItem => ({
    blockId,
    name: `Agent-${blockId}`,
    color: "#3b82f6",
    what: "asks you a question",
    since: 0,
    local: true,
    ...over,
});

function harness(opts: { enabled?: boolean; inFront?: boolean; focused?: string | null; paused?: boolean } = {}) {
    let items: AttentionItem[] = [];
    let t = 0;
    const state = {
        enabled: opts.enabled ?? true,
        inFront: opts.inFront ?? true,
        focused: opts.focused ?? null,
        paused: opts.paused ?? false,
    };
    // The one expiry timer the toast has set, if it still holds.
    let expiry: (() => void) | null = null;
    const pushed: NotificationType[] = [];
    const removed: string[] = [];
    const actions = new Map<string, () => void>();
    const revealed: string[] = [];
    const toast = createAttentionToast({
        items: () => items,
        enabled: () => state.enabled,
        paused: () => state.paused,
        inFront: () => state.inFront,
        focusedBlock: () => state.focused,
        push: (n) => pushed.push(n),
        remove: (id) => removed.push(id),
        setAction: (k, fn) => actions.set(k, fn),
        reveal: (b) => revealed.push(b),
        now: () => t,
        later: (fn) => {
            expiry = fn;
            return () => {
                if (expiry === fn) expiry = null;
            };
        },
    });
    return {
        expiry: () => expiry,
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
            expiresWithActions: true,
            expiration: START_GRACE_MS + TOAST_LIFETIME_MS,
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

    it("does not come back after a Go to agent button took it off the screen, which is no X", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1"), item("b2", { local: false })]);
        h.actions.get("attention:go:b2")!();
        expect(h.revealed).toEqual(["b2"]);
        h.wait(6000);
        h.set([item("b1"), item("b2", { local: false })]);
        expect(h.pushed).toHaveLength(1);
        h.set([item("b1"), item("b2", { local: false }), item("b3")]);
        expect(h.last()).toMatchObject({ title: "Agent-b3" });
    });

    it("does not push the toast again while what it says is unchanged (a screen reader would read it out again)", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.set([item("b1")]);
        h.wait(20_000);
        h.set([item("b1")]);
        expect(h.pushed).toHaveLength(1);
        h.set([item("b1", { what: "asks to approve a Bash call" })]);
        expect(h.pushed).toHaveLength(2);
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

    it("shows a request that came while the person was on its pane once they look away", () => {
        const h = harness({ focused: "b1" });
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        expect(h.pushed).toEqual([]);
        h.state.focused = "other";
        h.set([item("b1")]);
        expect(h.last()).toMatchObject({ title: "Agent-b1" });
    });

    it("does not count a request that leaves the list and returns at once as new", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.last().onClose!();
        h.set([]);
        h.wait(RETURN_MS - 1);
        h.set([item("b1")]);
        expect(h.pushed).toHaveLength(1);
        h.set([]);
        h.wait(RETURN_MS + 1);
        h.set([]);
        h.set([item("b1")]);
        expect(h.pushed).toHaveLength(2);
    });

    it("goes after its lifetime, and a list that changes later does not bring back what it said", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        expect(h.last()).not.toHaveProperty("persistent");
        h.expiry()!();
        h.wait(TOAST_LIFETIME_MS);
        h.set([item("b1")]);
        expect(h.pushed).toHaveLength(1);
        h.set([item("b1"), item("b2")]);
        expect(h.pushed).toHaveLength(2);
        expect(h.last()).toMatchObject({ title: "Agent-b2" });
    });

    it("sets no expiry timer once the toast is gone, and a new one starts its own", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        expect(h.expiry()).not.toBeNull();
        h.set([]);
        expect(h.expiry()).toBeNull();
        h.wait(RETURN_MS + 1);
        h.set([item("b2")]);
        expect(h.expiry()).not.toBeNull();
        expect(h.last().expiration).toBe(START_GRACE_MS + RETURN_MS + 1 + TOAST_LIFETIME_MS);
    });

    it("shows nothing while Do Not Disturb holds notices back, and does not bring those requests up when it ends", () => {
        const h = harness({ paused: true });
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        expect(h.pushed).toEqual([]);
        h.state.paused = false;
        h.set([item("b1")]);
        expect(h.pushed).toEqual([]);
        h.set([item("b1"), item("b2")]);
        expect(h.last()).toMatchObject({ title: "2 agents need you" });
    });

    it("takes the toast off the screen when Do Not Disturb begins", () => {
        const h = harness();
        h.wait(START_GRACE_MS);
        h.set([item("b1")]);
        h.state.paused = true;
        h.set([item("b1")]);
        expect(h.removed).toEqual([ATTENTION_TOAST_ID]);
    });

    it("knows Do Not Disturb as srv does: a pause that has not run out, unless attention is let through", () => {
        expect(noticesPaused(2000, false, 1000)).toBe(true);
        expect(noticesPaused(2000, true, 1000)).toBe(false);
        expect(noticesPaused(1000, false, 1000)).toBe(false);
        expect(noticesPaused(undefined, false, 1000)).toBe(false);
        expect(noticesPaused(0, false, 1000)).toBe(false);
    });

    it("draws the agent's name in the colour the theme can read", () => {
        const toast = buildToast(
            [item("b1", { color: "#3b82f6" })],
            () => {},
            () => {},
            (hex) => `tint(${hex})`
        );
        expect(toast.titleColor).toBe("tint(#3b82f6)");
    });
});
