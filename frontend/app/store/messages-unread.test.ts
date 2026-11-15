// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { messagesUnreadFor, publishMessagesUnread, resetMessagesUnread } from "./messages-unread";

afterEach(resetMessagesUnread);

describe("messages-unread", () => {
    it("says what a pane's panel holds, per pane", () => {
        publishMessagesUnread("a", { unread: 3, needsYou: false });
        publishMessagesUnread("b", { unread: 1, needsYou: true });
        expect(messagesUnreadFor("a")).toEqual({ unread: 3, needsYou: false });
        expect(messagesUnreadFor("b")).toEqual({ unread: 1, needsYou: true });
        expect(messagesUnreadFor("c")).toBeNull();
    });

    it("has nothing for a pane with nothing unread, even when something waits on the person", () => {
        publishMessagesUnread("a", { unread: 0, needsYou: true });
        expect(messagesUnreadFor("a")).toBeNull();
    });

    it("takes a pane's entry away and leaves the others", () => {
        publishMessagesUnread("a", { unread: 2, needsYou: false });
        publishMessagesUnread("b", { unread: 2, needsYou: false });
        publishMessagesUnread("a", null);
        publishMessagesUnread("gone", null);
        expect(messagesUnreadFor("a")).toBeNull();
        expect(messagesUnreadFor("b")?.unread).toBe(2);
    });
});
