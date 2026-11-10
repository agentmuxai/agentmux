// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import type { AuditEntry } from "@/types/rpc/AuditEntry";

const settings: Record<string, unknown> = {};
vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: (key: string) => () => settings[key] }));

import type { PlayedSound } from "@/app/notification/sound/sound-history";
import { encodeSoundRef, kindOfSound, lastSound, lastSoundLine, litRowId, parseSoundRef, rowOf, soundFor, soundIsOn, soundText } from "./audit-sound";

const pane = (block_id: string) => ({ pane: { block_id } });
function entry(over: Partial<AuditEntry>): AuditEntry {
    return { id: "e", ts_ms: 10_000, kind: "tool", summary: "Bash ls", agent_id: "korp", links: pane("blk-korp"), ...over };
}
const played = (over: Partial<PlayedSound>): PlayedSound => ({ blockId: "blk-korp", event: "tool", tool: "Bash", at: 10_100, ...over });

describe("the sound a row makes", () => {
    it("is the one the sound service plays for it", () => {
        expect(soundFor(entry({ kind: "tool" }))).toBe("tool");
        expect(soundFor(entry({ kind: "turn", outcome: "success" }))).toBe("agent.turn.complete");
        expect(soundFor(entry({ kind: "turn", outcome: "error_max_turns" }))).toBe("agent.turn.error");
        expect(soundFor(entry({ kind: "jekt", summary: "a → b: delivered, 9 bytes" }))).toBe("agent.message.accepted");
        expect(soundFor(entry({ kind: "jekt", summary: "a → b: failed (refused), 9 bytes" }))).toBe("agent.message.rejected");
    });

    it("is none for the sender's half, a held message and the kinds that make no sound", () => {
        expect(soundFor(entry({ kind: "jekt", provenance: { sent_only: true } }))).toBeNull();
        expect(soundFor(entry({ kind: "jekt", outcome: "held", summary: "a → b: held, 9 bytes" }))).toBeNull();
        for (const kind of ["action", "registry", "agent_start", "agent_end", "thought"]) expect(soundFor(entry({ kind }))).toBeNull();
    });

    it("shows no speaker for a muted sound", () => {
        expect(soundIsOn("tool")).toBe(true);
        settings["notify:tooltones:enabled"] = false;
        expect(soundIsOn("tool")).toBe(false);
        expect(soundIsOn("agent.turn.complete")).toBe(true);
        settings["notify:sound:agent.turn.complete"] = false;
        expect(soundIsOn("agent.turn.complete")).toBe(false);
        delete settings["notify:tooltones:enabled"];
        delete settings["notify:sound:agent.turn.complete"];
        settings["notify:sounds:enabled"] = false;
        expect(soundIsOn("tool")).toBe(false);
        delete settings["notify:sounds:enabled"];
    });
});

describe("the row a sound came from", () => {
    const rows = [
        entry({ id: "new", ts_ms: 10_050, links: pane("blk-korp") }),
        entry({ id: "other-pane", ts_ms: 10_060, links: pane("blk-loap") }),
        entry({ id: "turn", ts_ms: 10_070, kind: "turn", outcome: "success" }),
        entry({ id: "old", ts_ms: 5_000 }),
    ];

    it("is the newest of the same pane and kind of sound within two seconds", () => {
        expect(rowOf(rows, played({}))?.id).toBe("new");
        expect(rowOf(rows, played({ event: "agent.turn.complete" }))?.id).toBe("turn");
        expect(rowOf(rows, played({ blockId: "blk-nobody" }))).toBeUndefined();
        expect(rowOf(rows, played({ at: 20_000 }))).toBeUndefined();
    });

    it("for a jekt is the receiver's pane", () => {
        const jekt = entry({ id: "j", kind: "jekt", links: {}, to: { agent_id: "loap", links: pane("blk-loap") }, summary: "korp → loap: delivered, 9 bytes" });
        expect(rowOf([jekt], played({ blockId: "blk-loap", event: "agent.message.accepted" }))?.id).toBe("j");
        expect(rowOf([jekt], played({ blockId: "blk-korp", event: "agent.message.accepted" }))).toBeUndefined();
    });

    it("lights only within three seconds of the sound", () => {
        expect(litRowId(rows, [played({})], 10_100 + 2_999)).toBe("new");
        expect(litRowId(rows, [played({})], 10_100 + 3_000)).toBeUndefined();
    });

    it("says who made the last sound, and nothing after a minute", () => {
        expect(lastSoundLine(rows, [played({})], 14_100)).toEqual({ text: "♪ korp · Bash · 4s ago", blockId: "blk-korp" });
        expect(lastSoundLine(rows, [played({})], 10_100 + 60_001)).toBeNull();
        expect(lastSoundLine(rows, [], 10_000)).toBeNull();
    });
});

describe("the last sound, for the status bar and the Audit's bar", () => {
    it("is the newest sound for a minute, and says who when it is known", () => {
        expect(lastSound([played({ at: 1_000 }), played({ at: 5_000 })], 6_000)?.at).toBe(5_000);
        expect(lastSound([played({ at: 5_000 })], 5_000 + 60_001)).toBeUndefined();
        expect(lastSound([], 5_000)).toBeUndefined();
        expect(soundText(played({ at: 10_000 }), "korp", 14_000)).toBe("♪ korp · Bash · 4s ago");
        expect(soundText(played({ at: 10_000, event: "agent.turn.complete", tool: undefined }), undefined, 14_000)).toBe("♪ Agent turn completed · 4s ago");
    });

    it("names the kind of row a sound comes from, so the Audit can show that kind", () => {
        expect(kindOfSound("tool")).toBe("tool");
        expect(kindOfSound("agent.turn.complete")).toBe("turn");
        expect(kindOfSound("agent.turn.error")).toBe("turn");
        expect(kindOfSound("agent.message.accepted")).toBe("jekt");
        expect(kindOfSound("agent.message.rejected")).toBe("jekt");
    });

    it("travels in the pane's meta and comes back the same, and a malformed one is none", () => {
        const sound = played({ at: 123_456 });
        expect(parseSoundRef(encodeSoundRef(sound, 99))).toEqual({ blockId: "blk-korp", event: "tool", at: 123_456, asked: 99 });
        for (const raw of [undefined, 7, "", "blk|tool", "|tool|1|2", "blk|tool|x|2"]) expect(parseSoundRef(raw)).toBeUndefined();
    });
});
