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
import { afterEach, describe, expect, it } from "vitest";

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

    it("the flushing item gets the agent-pending-item-flushing class", () => {
        const [pending] = createSignal<PendingMessage[]>([msg({ flushing: true })]);
        const { container } = render(() => (
            <PendingMessagesPanel pendingMessages={pending} />
        ));
        expect(container.querySelector(".agent-pending-item-flushing")).not.toBeNull();
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
