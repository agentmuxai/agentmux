// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import { formatLogLine, sanitizeLogTextForTerminal, useShellLogBridge } from "./useShellLogBridge";

describe("sanitizeLogTextForTerminal", () => {
    it("strips CSI sequences: colors, cursor moves, erases", () => {
        expect(sanitizeLogTextForTerminal("\x1b[31mred\x1b[0m \x1b[2J\x1b[10;5Hok")).toBe("red ok");
    });

    it("strips OSC sequences such as a window-title change", () => {
        expect(sanitizeLogTextForTerminal("\x1b]0;title\x07after")).toBe("after");
    });

    it("drops stray control bytes but keeps tabs", () => {
        expect(sanitizeLogTextForTerminal("a\x07b\x1b c\rd\te\x7f")).toBe("ab cd\te");
    });

    it("strips ESC c (full terminal reset) as a sequence, not just its ESC", () => {
        expect(sanitizeLogTextForTerminal("a\x1bcb")).toBe("ab");
    });

    it("turns bare newlines into CRLF so lines don't staircase", () => {
        expect(sanitizeLogTextForTerminal("one\ntwo\nthree")).toBe("one\r\ntwo\r\nthree");
    });

    it("leaves plain text alone", () => {
        expect(sanitizeLogTextForTerminal("cargo test: 12 passed")).toBe("cargo test: 12 passed");
    });
});

describe("formatLogLine", () => {
    it("tags and colors by level: grey info, yellow warn, red error", () => {
        expect(formatLogLine("system", "hi")).toBe("\x1b[90m[system] hi\x1b[0m");
        expect(formatLogLine("system", "hi", "warn")).toBe("\x1b[33m[system] hi\x1b[0m");
        expect(formatLogLine("system", "hi", "error")).toBe("\x1b[31m[system] hi\x1b[0m");
    });

    it("sanitizes the text it wraps", () => {
        expect(formatLogLine("system", "\x1b[2Jx")).toBe("\x1b[90m[system] x\x1b[0m");
    });
});

describe("useShellLogBridge", () => {
    const withBridge = (fn: (bridge: ReturnType<typeof useShellLogBridge>) => void) =>
        createRoot((dispose) => {
            fn(useShellLogBridge());
            dispose();
        });

    it("writes only system-tagged lines; the rest is kept out of the shell", () => {
        withBridge((b) => {
            const out: string[] = [];
            b.onTermReady((t) => out.push(t));
            b.log("cli", "checking for claude...");
            b.log("system", "$ ls");
            expect(out).toEqual([formatLogLine("system", "$ ls")]);
        });
    });

    it("replays lines logged while no terminal was mounted, once", () => {
        withBridge((b) => {
            b.log("system", "before");
            const first: string[] = [];
            b.onTermReady((t) => first.push(t));
            expect(first).toEqual([formatLogLine("system", "before")]);

            b.onTermDispose();
            b.log("system", "while closed");
            const second: string[] = [];
            b.onTermReady((t) => second.push(t));
            // Only what the first terminal never saw.
            expect(second).toEqual([formatLogLine("system", "while closed")]);
        });
    });

    it("stops writing live after the terminal is disposed or cleared", () => {
        withBridge((b) => {
            const out: string[] = [];
            b.onTermReady((t) => out.push(t));
            b.onTermDispose();
            b.log("system", "after dispose");
            b.onTermReady((t) => out.push(t));
            b.clearTermWrite();
            b.log("system", "after clear");
            expect(out).toEqual([formatLogLine("system", "after dispose")]);
        });
    });
});
