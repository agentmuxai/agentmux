// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { forgetRefusedSend, refusedSendOf, rememberRefusedSend } from "./refused-send";

describe("refused sends", () => {
    afterEach(() => forgetRefusedSend("b1"));

    it("keeps both messages when a second send is refused before Move here", () => {
        const att = { path: "a.png" } as unknown as AttachmentRef;
        rememberRefusedSend("b1", { text: "first", attachments: [] });
        rememberRefusedSend("b1", { text: "second", attachments: [att] });
        expect(refusedSendOf("b1")).toEqual({ text: "first\n\nsecond", attachments: [att] });
    });

    it("does not repeat the kept message when its resend is refused again", () => {
        rememberRefusedSend("b1", { text: "first", attachments: [] });
        rememberRefusedSend("b1", { text: "first", attachments: [] });
        expect(refusedSendOf("b1")?.text).toBe("first");
    });

    it("keeps both payloads when the same text is refused with different attachments", () => {
        const one = { path: "1.png" } as unknown as AttachmentRef;
        const two = { path: "2.png" } as unknown as AttachmentRef;
        rememberRefusedSend("b1", { text: "look", attachments: [one] });
        rememberRefusedSend("b1", { text: "look", attachments: [two] });
        expect(refusedSendOf("b1")?.attachments).toEqual([one, two]);
    });

    it("forgets a kept message", () => {
        rememberRefusedSend("b1", { text: "x", attachments: [] });
        forgetRefusedSend("b1");
        expect(refusedSendOf("b1")).toBeUndefined();
    });
});
