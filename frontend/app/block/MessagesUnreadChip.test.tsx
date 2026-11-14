// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { publishMessagesUnread, resetMessagesUnread } from "@/app/store/messages-unread";
import { chipTitle, MessagesUnreadChip } from "./MessagesUnreadChip";

afterEach(() => {
    cleanup();
    resetMessagesUnread();
});

const chip = (c: HTMLElement) => c.querySelector<HTMLElement>(".messages-unread-chip");

describe("MessagesUnreadChip", () => {
    it("draws nothing while the pane has nothing unread", () => {
        const { container } = render(() => <MessagesUnreadChip blockId="p1" />);
        expect(chip(container)).toBeNull();
    });

    it("shows the number the header button shows, and names it for a screen reader and a tooltip", () => {
        publishMessagesUnread("p1", { unread: 3, needsYou: false });
        const { container } = render(() => <MessagesUnreadChip blockId="p1" />);
        expect(chip(container)?.textContent).toBe("3");
        expect(chip(container)?.getAttribute("aria-label")).toBe("Messages, 3 unread");
        expect(container.querySelector(".ui-pill")?.getAttribute("title")).toBe("Messages, 3 unread");
    });

    it("caps the count and says when something needs the person", () => {
        publishMessagesUnread("p1", { unread: 120, needsYou: true });
        const { container } = render(() => <MessagesUnreadChip blockId="p1" />);
        expect(chip(container)?.textContent).toBe("99+");
        expect(chip(container)?.getAttribute("aria-label")).toBe("Messages, 120 unread, needs you");
        expect(chipTitle(1, false)).toBe("Messages, 1 unread");
    });

    it("follows the pane's count and goes when it is read", () => {
        const { container } = render(() => <MessagesUnreadChip blockId="p1" />);
        publishMessagesUnread("p1", { unread: 1, needsYou: false });
        expect(chip(container)?.textContent).toBe("1");
        publishMessagesUnread("p1", { unread: 2, needsYou: false });
        expect(chip(container)?.textContent).toBe("2");
        publishMessagesUnread("p1", { unread: 0, needsYou: false });
        expect(chip(container)).toBeNull();
    });

    it("is of its own pane only", () => {
        publishMessagesUnread("p2", { unread: 5, needsYou: false });
        const { container } = render(() => <MessagesUnreadChip blockId="p1" />);
        expect(chip(container)).toBeNull();
    });
});
