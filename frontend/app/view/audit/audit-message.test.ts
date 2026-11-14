// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { messageAsk, newestAsk, soundAsk } from "./audit-focus";
import { encodeMessageRef, parseMessageRef, rowOfMessage } from "./audit-message";

const entry = (over: Partial<AuditEntry>): AuditEntry =>
    ({ id: "e", ts_ms: 1000, kind: "jekt", summary: "", links: {}, ...over }) as AuditEntry;

describe("a message asked of the Audit", () => {
    it("round-trips through the pane's meta, with an id of any text", () => {
        const raw = encodeMessageRef("1794686210090-123796-20", 5000, 9000);
        expect(parseMessageRef(raw)).toEqual({ msgId: "1794686210090-123796-20", at: 5000, asked: 9000 });
        expect(parseMessageRef(encodeMessageRef("a|b", 1, 2))?.msgId).toBe("a|b");
    });

    it("reads nothing from what is not an ask", () => {
        expect(parseMessageRef(undefined)).toBeUndefined();
        expect(parseMessageRef(42)).toBeUndefined();
        expect(parseMessageRef("x|1|id")).toBeUndefined();
        expect(parseMessageRef("1|2|")).toBeUndefined();
    });

    it("finds the jekt row that carries the id, and no other kind of row", () => {
        const rows = [
            entry({ id: "tool", kind: "tool", provenance: { msg_id: "m1" } }),
            entry({ id: "other", provenance: { msg_id: "m2" } }),
            entry({ id: "mine", provenance: { msg_id: "m1" } }),
            entry({ id: "bare" }),
        ];
        expect(rowOfMessage(rows, "m1")?.id).toBe("mine");
        expect(rowOfMessage(rows, "nope")).toBeUndefined();
    });

    it("asks for the jekt kind, from when the message arrived", () => {
        const ask = messageAsk({ msgId: "m1", at: 5000, asked: 7 });
        expect(ask).toMatchObject({ asked: 7, at: 5000, kind: "jekt" });
        expect(ask?.find([entry({ id: "mine", provenance: { msg_id: "m1" } })])?.id).toBe("mine");
        expect(messageAsk(undefined)).toBeUndefined();
    });

    it("follows the later of a sound and a message", () => {
        const sound = soundAsk({ blockId: "b", event: "tool", at: 1, asked: 5 });
        const message = messageAsk({ msgId: "m1", at: 1, asked: 8 });
        expect(newestAsk(sound, message)).toBe(message);
        expect(newestAsk(messageAsk({ msgId: "m1", at: 1, asked: 3 }), sound)).toBe(sound);
        expect(newestAsk(sound, undefined)).toBe(sound);
        expect(newestAsk(undefined, undefined)).toBeUndefined();
    });
});
