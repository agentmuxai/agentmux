// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Coverage for the `flushing` copy split
 * (docs/specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1) —
 * once TurnEnd marks a busy-enqueued entry `flushing`, the panel must stop
 * claiming it "sends at the agent's next step" (the turn already ended;
 * only the async accept ack is left) and switch to honest "Sending…" copy.
 * Not a full component smoke test — the FIFO ordering / accept-removal
 * behavior is covered at the reducer level (agent-pane-state/reducer.input.test.ts).
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PendingMessagesPanel } from "./PendingMessagesPanel";
import type { PendingMessage } from "../../../agent-model/state";

afterEach(() => {
    cleanup();
});

const msg = (overrides: Partial<PendingMessage>): PendingMessage => ({
    id: "m1",
    text: "hello",
    createdAt: 100,
    enqueuedWhileBusy: true,
    ...overrides,
});

describe("PendingMessagesPanel — flushing copy split", () => {
    it("renders nothing when there are no busy-enqueued messages", () => {
        const [pending] = createSignal<PendingMessage[]>([
            msg({ enqueuedWhileBusy: false }),
        ]);
        const { container } = render(() => (
            <PendingMessagesPanel pendingMessages={pending} />
        ));
        expect(container.querySelector(".agent-pending-zone")).toBeNull();
    });

    it("shows the 'Queued — sends at the agent's next step' copy while genuinely still queued", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({})]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Queued — sends at the agent's next step/)).toBeTruthy();
        expect(screen.queryByText(/Sending — reaching the agent/)).toBeNull();
    });

    it("switches to 'Sending — reaching the agent any moment' once flushing", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true })]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Sending — reaching the agent any moment/)).toBeTruthy();
        expect(screen.queryByText(/sends at the agent's next step/)).toBeNull();
    });

    it("prefers the queued copy (with the still-queued count) when the two groups are mixed", () => {
        const [pending] = createSignal<PendingMessage[]>([
            msg({ id: "a", flushing: true }),
            msg({ id: "b", flushing: false }),
        ]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        // Still-open count only (1), not the total (2) — the flushing entry
        // is no longer "queued" in the sense this copy describes.
        expect(screen.getByText(/Queued — sends at the agent's next step.*\(1 message\)/)).toBeTruthy();
    });

    // srv queued the delivered message behind the agent's live process (a command sent to the background keeps it
    // alive): "any moment" is not true, and the person is told what it waits for and how many are ahead.
    it("says the agent's command is still running, not 'any moment', once srv queued it behind the process", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true, waiting: { ahead: 0 } })]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Waiting \(1 message\) — the agent's command is still running/)).toBeTruthy();
        expect(screen.queryByText(/reaching the agent any moment/)).toBeNull();
    });

    it("names how many messages are ahead of the first waiting one", () => {
        const [pending] = createSignal<PendingMessage[]>([
            msg({ id: "a", flushing: true, waiting: { ahead: 2 } }),
            msg({ id: "b", flushing: true, waiting: { ahead: 3 } }),
        ]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Waiting \(2 messages, 2 ahead\) — the agent's command is still running/)).toBeTruthy();
    });

    // The header must follow the count and `ahead` while the panel stays in the waiting state (no clearing between).
    it("updates the waiting header when the count and ahead change while it stays waiting", () => {
        const [pending, setPending] = createSignal<PendingMessage[]>([msg({ id: "a", flushing: true, waiting: { ahead: 0 } })]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Waiting \(1 message\) — /)).toBeTruthy();
        setPending([
            msg({ id: "a", flushing: true, waiting: { ahead: 0 } }),
            msg({ id: "b", flushing: true, waiting: { ahead: 1 } }),
            msg({ id: "c", flushing: true, waiting: { ahead: 2 } }),
        ]);
        expect(screen.getByText(/Waiting \(3 messages\) — /)).toBeTruthy();
        setPending([
            msg({ id: "b", flushing: true, waiting: { ahead: 1 } }),
            msg({ id: "c", flushing: true, waiting: { ahead: 2 } }),
        ]);
        expect(screen.getByText(/Waiting \(2 messages, 1 ahead\) — /)).toBeTruthy();
    });

    it("keeps 'any moment' for a flushing message srv did not report as waiting", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true })]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(screen.getByText(/Sending — reaching the agent any moment/)).toBeTruthy();
        expect(screen.queryByText(/Waiting — the agent/)).toBeNull();
    });

    it("the flushing item gets the agent-pending-item-flushing class", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true })]);
        const { container } = render(() => (
            <PendingMessagesPanel pendingMessages={pending} />
        ));
        expect(container.querySelector(".agent-pending-item-flushing")).not.toBeNull();
    });
});

describe("PendingMessagesPanel — one timer for the queue", () => {
    const timers = (container: HTMLElement) => Array.from(container.querySelectorAll(".agent-pending-timer")).map((n) => n.textContent);

    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("counts how long the oldest message has waited, once for however many are queued", () => {
        vi.useFakeTimers();
        vi.setSystemTime(100_000);
        const [pending] = createSignal<PendingMessage[]>([
            msg({ id: "a", createdAt: 100_000 - 65_000, flushing: true, waiting: { ahead: 0 } }),
            msg({ id: "b", createdAt: 100_000 - 20_000, flushing: true, waiting: { ahead: 1 } }),
            msg({ id: "c", createdAt: 100_000 - 5_000, flushing: true, waiting: { ahead: 2 } }),
        ]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        vi.advanceTimersByTime(1000);
        expect(timers(container)).toEqual(["1m 6s"]);
        vi.advanceTimersByTime(4000);
        expect(timers(container)).toEqual(["1m 10s"]);
    });

    it("shows no clock for a message that has not waited a second, so one that goes straight through never flashes 0s", () => {
        vi.useFakeTimers();
        vi.setSystemTime(100_000);
        const [pending] = createSignal<PendingMessage[]>([msg({ createdAt: 100_000 - 200 })]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(timers(container)).toEqual([]);
        vi.advanceTimersByTime(1000);
        expect(timers(container)).toEqual(["1s"]);
    });

    it("follows the oldest message when it leaves the queue", () => {
        vi.useFakeTimers();
        vi.setSystemTime(100_000);
        const [pending, setPending] = createSignal<PendingMessage[]>([
            msg({ id: "a", createdAt: 100_000 - 60_000 }),
            msg({ id: "b", createdAt: 100_000 - 10_000 }),
        ]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        vi.advanceTimersByTime(1000);
        expect(timers(container)).toEqual(["1m 1s"]);
        setPending([msg({ id: "b", createdAt: 100_000 - 10_000 })]);
        expect(timers(container)).toEqual(["11s"]);
    });

    it("stops ticking once the queue is empty", () => {
        vi.useFakeTimers();
        const [pending, setPending] = createSignal<PendingMessage[]>([msg({})]);
        render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(vi.getTimerCount()).toBe(1);
        setPending([]);
        expect(vi.getTimerCount()).toBe(0);
    });
});

describe("PendingMessagesPanel — image attachments", () => {
    const attachments = [
        { id: "a".repeat(64), name: "shot.png" },
        { id: "b".repeat(64), name: "diagram.png" },
    ];

    it("shows a queued message's images above its text, as a sent message does", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ text: "look at these", attachments })]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        const item = container.querySelector(".agent-pending-item")!;
        expect(item.querySelector(".agent-attachment-strip")).not.toBeNull();
        expect(item.querySelectorAll(".agent-attachment-tile").length).toBe(2);
        expect(item.querySelector("pre")?.textContent).toBe("look at these");
        // Strip first, then the words, like UserMessageBlock.
        const strip = item.querySelector(".agent-attachment-strip")!;
        expect(strip.compareDocumentPosition(item.querySelector("pre")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("shows an images-only message without an empty text block", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ text: "", attachments })]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(container.querySelector(".agent-attachment-strip")).not.toBeNull();
        expect(container.querySelector(".agent-pending-item pre")).toBeNull();
    });

    it("shows no strip for a text-only message", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({})]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(container.querySelector(".agent-attachment-strip")).toBeNull();
    });

    it("keeps the images while the entry is being delivered", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true, attachments })]);
        const { container } = render(() => <PendingMessagesPanel pendingMessages={pending} />);
        expect(container.querySelectorAll(".agent-pending-item-flushing .agent-attachment-tile").length).toBe(2);
    });
});
