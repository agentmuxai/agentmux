// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { committed, conflictText, isPlainKey, ownWhen, PLAIN_KEY_GUARD, propose, recorderStatus, verdictOf, whereChoices, type RecorderOptions } from "./keyboard-recorder-state";

const OPTS: RecorderOptions = { keepCurrent: false, where: "", allowPlain: false };

/** The setting after `steps` are recorded for `command` on Windows, with the verdict the person would see. */
function record(command: string, steps: string[], options: Partial<RecorderOptions> = {}, entries: unknown[] = [], confirmed = false) {
    const p = propose(entries, command, steps, { ...OPTS, ...options }, "other");
    if (!p) throw new Error("nothing recorded");
    return { p, verdict: verdictOf(p, confirmed), entries: (replace = false) => committed(entries, command, p, "other", replace) };
}

describe("propose", () => {
    it("has nothing to say until a key is recorded", () => {
        expect(propose([], "tab:new", [], OPTS, "other")).toBeNull();
    });

    it("replaces the command's keys, written as a diff from the defaults", () => {
        const r = record("tab:new", ["ctrl+shift+y"]);
        expect(r.verdict).toBe("save");
        expect(r.entries()).toEqual([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
        ]);
    });

    it("keeps the current keys when asked", () => {
        expect(record("tab:new", ["ctrl+shift+y"], { keepCurrent: true }).entries()).toEqual([{ command: "tab:new", key: "ctrl+shift+y", platform: "other" }]);
    });

    it("records a two-step shortcut as one key with a space", () => {
        const r = record("tab:new", ["ctrl+k", "ctrl+n"]);
        expect(r.p.key).toBe("ctrl+k ctrl+n");
        expect(r.verdict).toBe("save");
    });
});

describe("collisions", () => {
    it("offers Replace for a key another command has where both work, and takes it from that command", () => {
        const r = record("tab:new", ["ctrl+shift+w"]);
        expect(r.verdict).toBe("replace");
        expect(r.p.blocking.map((c) => c.command)).toEqual(["pane:close"]);
        expect(conflictText(r.p.blocking[0], "New tab", "other")).toBe("Ctrl+Shift+W is already Close pane, which works in the same places as New tab.");
        expect(r.entries(true)).toEqual(
            expect.arrayContaining([
                { command: "-pane:close", platform: "other" },
                { command: "tab:new", key: "ctrl+shift+w", platform: "other" },
            ])
        );
    });

    it("lets a key be kept beside another command's when it works somewhere else", () => {
        // Copy in a terminal has ctrl+shift+c; everywhere would overlap it, "not in a terminal" does not.
        expect(record("tab:new", ["ctrl+shift+c"]).verdict).toBe("replace");
        const apart = record("tab:new", ["ctrl+shift+c"], { where: "!terminalFocus" });
        expect(apart.verdict).toBe("save");
        expect(apart.entries()).toContainEqual({ command: "tab:new", key: "ctrl+shift+c", when: "!terminalFocus", platform: "other" });
    });

    it("works where the command works today unless the person says otherwise", () => {
        expect(ownWhen("tab:new")).toBe("");
        expect(ownWhen("term:copy")).toBe("terminalFocus && viewType == term");
        expect(whereChoices("tab:new")[0]).toEqual({ value: "", label: "Everywhere" });
        expect(whereChoices("tab:new")).toHaveLength(7);
        // Its own place is not one of the choices: it is offered first.
        expect(whereChoices("agent:focusComposer")[0]).toEqual({ value: "viewType == agent", label: "As it works today" });
    });
});

describe("keys the system takes", () => {
    it("cannot be saved when the system takes them before the app", () => {
        const r = record("tab:new", ["alt+F4"]);
        expect(r.verdict).toBe("stop");
        expect(r.p.blocked?.reason).toMatch(/Alt\+F4/);
    });

    it("needs the person's word when the desktop often takes them", () => {
        expect(record("tab:new", ["ctrl+alt+t"]).verdict).toBe("confirm");
        expect(record("tab:new", ["ctrl+alt+t"], {}, [], true).verdict).toBe("save");
    });
});

describe("plain keys", () => {
    it("tells a bare printable key from the rest", () => {
        expect(isPlainKey("t", "other")).toBe(true);
        expect(isPlainKey("shift+a", "other")).toBe(true);
        expect(isPlainKey("code:Space", "other")).toBe(true);
        expect(isPlainKey("5", "other")).toBe(true);
        expect(isPlainKey("ctrl+t", "other")).toBe(false);
        expect(isPlainKey("alt+t", "other")).toBe(false);
        expect(isPlainKey("F6", "other")).toBe(false);
        expect(isPlainKey("ArrowUp", "other")).toBe(false);
    });

    it("is refused until allowed, then saved only outside text fields and terminals", () => {
        const refused = record("tab:new", ["t"]);
        expect(refused.p.refusedPlain).toBe(true);
        expect(refused.verdict).toBe("stop");
        const allowed = record("tab:new", ["t"], { allowPlain: true });
        expect(allowed.verdict).toBe("save");
        expect(allowed.entries()).toContainEqual({ command: "tab:new", key: "t", when: PLAIN_KEY_GUARD, platform: "other" });
    });

    it("looks at the first step of a chord only", () => {
        expect(record("tab:new", ["ctrl+k", "t"]).p.refusedPlain).toBe(false);
        expect(record("tab:new", ["t", "ctrl+k"]).p.refusedPlain).toBe(true);
    });
});

describe("recorderStatus", () => {
    it("says what to do, then what the key is and what stands in its way", () => {
        expect(recorderStatus("New tab", null, null, "other")).toBe("Press the new shortcut for New tab. Escape cancels.");
        const ok = record("tab:new", ["ctrl+shift+y"]);
        expect(recorderStatus("New tab", ok.p, ok.verdict, "other")).toBe("Ctrl+Shift+Y. Enter saves, or press another key to make it a two-step shortcut.");
        const taken = record("tab:new", ["ctrl+shift+w"]);
        expect(recorderStatus("New tab", taken.p, taken.verdict, "other")).toMatch(/taken by Close pane/);
        const plain = record("tab:new", ["t"]);
        expect(recorderStatus("New tab", plain.p, plain.verdict, "other")).toMatch(/plain key/);
    });
});
