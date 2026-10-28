// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { afterEach, describe, expect, it, vi } from "vitest";

const restoreToComposer = vi.hoisted(() => vi.fn());
const restoreRefs = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("../composer-restore", () => ({ restoreToComposer }));
vi.mock("../attachments/attachment-draft", () => ({ getAttachmentDraft: () => ({ restoreRefs }) }));

import { forgetRefusedSend, refusedSendOf, rememberRefusedSend } from "./refused-send";
import { restoreRefusedSend } from "./refused-send-restore";

afterEach(() => {
    forgetRefusedSend("b1");
    vi.clearAllMocks();
});

const att: AttachmentRef = { id: "a1", name: "plan.md" };

describe("restoreRefusedSend", () => {
    it("puts the refused text back in the composer and its attachments back in the tray, and forgets it", () => {
        rememberRefusedSend("b1", { text: "are you there", attachments: [att] });
        restoreRefusedSend("b1");
        expect(restoreToComposer).toHaveBeenCalledWith("b1", "are you there");
        expect(restoreRefs).toHaveBeenCalledWith([att]);
        expect(refusedSendOf("b1")).toBeUndefined();
    });

    it("leaves the tray alone for a message with no attachments", () => {
        rememberRefusedSend("b1", { text: "hello", attachments: [] });
        restoreRefusedSend("b1");
        expect(restoreToComposer).toHaveBeenCalledWith("b1", "hello");
        expect(restoreRefs).not.toHaveBeenCalled();
    });

    it("does nothing when no message was refused, and restores a message only once", () => {
        restoreRefusedSend("b1");
        expect(restoreToComposer).not.toHaveBeenCalled();
        rememberRefusedSend("b1", { text: "hello", attachments: [] });
        restoreRefusedSend("b1");
        restoreRefusedSend("b1");
        expect(restoreToComposer).toHaveBeenCalledTimes(1);
    });
});
