// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a translator takes from a record's `amx` (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md
 * §6.1, §10 S4): only an origin srv verified is read; anything else is untyped.
 */

import { describe, expect, test } from "vitest";
import { ClaudeTranslator } from "./claude-translator";
import { originFields, recordOrigin } from "./translator";

const record = (amx: unknown, content = "hi") => ({ type: "user", message: { role: "user", content }, ...(amx === undefined ? {} : { amx }) });
const verified = (origin: unknown, extra: object = {}) => ({ v: 1, verified: true, origin, ...extra });

describe("recordOrigin", () => {
    test("reads a verified origin and its notes", () => {
        expect(recordOrigin(record(verified({ kind: "agent", from: "agent1" }, { notes: ["[AgentMux] a", 3] })))).toEqual({
            origin: { kind: "agent", from: "agent1" },
            notes: ["[AgentMux] a"],
        });
    });

    test.each([
        ["no amx", undefined],
        ["not verified", { v: 1, origin: { kind: "user" } }],
        ["verified false", { v: 1, verified: false, origin: { kind: "user" } }],
        ["an unknown amx.v", verified({ kind: "user" }, { v: 2 })],
        ["an unknown kind", verified({ kind: "operator" })],
        ["a malformed origin", verified("user")],
        ["no origin", { v: 1, verified: true }],
    ])("S4: %s is untyped", (_name, amx) => {
        expect(recordOrigin(record(amx))).toBeUndefined();
    });
});

describe("originFields", () => {
    test("a provider srv records for marks an untyped record; the others do not", () => {
        expect(originFields(record(undefined), true)).toEqual({ untyped: true });
        expect(originFields(record(undefined), false)).toEqual({});
    });

    test("a typed record carries its origin, and its notes only when it has some", () => {
        expect(originFields(record(verified({ kind: "user" })), true)).toEqual({ origin: { kind: "user" } });
        expect(originFields(record(verified({ kind: "user" }, { notes: ["n"] })), true)).toEqual({ origin: { kind: "user" }, notes: ["n"] });
    });
});

describe("the Claude translator", () => {
    const translate = (rec: unknown) => new ClaudeTranslator({ replay: true }).translate(rec);

    test("a stored user message becomes a user_message with its verified origin", () => {
        const events = translate(record(verified({ kind: "schedule", from: "cron" })));
        expect(events).toMatchObject([{ type: "user_message", message: "hi", origin: { kind: "schedule", from: "cron" } }]);
    });

    test("an unverified one is marked untyped", () => {
        const events = translate(record({ origin: { kind: "user" } }));
        expect(events).toMatchObject([{ type: "user_message", untyped: true }]);
        expect((events[0] as { origin?: unknown }).origin).toBeUndefined();
    });
});
