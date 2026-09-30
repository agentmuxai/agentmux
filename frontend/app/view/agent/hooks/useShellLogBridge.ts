// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 2).

import { createSignal } from "solid-js";
import { useActivityLog } from "./useActivityLog";

// Matches a CSI or OSC ANSI escape sequence (the standard sindresorhus/ansi-regex
// pattern). Used by sanitizeLogTextForTerminal below to strip escape sequences
// out of arbitrary text (e.g. a bang command's subprocess stdout/stderr) before
// it's wrapped in formatLogLine's own SGR color codes and written into the live
// shell Terminal — otherwise embedded sequences in that text could move the
// cursor, recolor arbitrary regions, or otherwise corrupt the shared terminal's
// rendered state (this text is not our own trusted output; it's shell-command
// output the user chose to run).
//
// OSC is matched first as ESC ] (or its 8-bit form) through the nearest BEL
// or ST (ESC-backslash or 0x9C), whatever text it carries: the older pattern alone
// allowed only a narrow character set, so a window title with a space, or an
// ST terminator, left text behind. Same approach as current ansi-regex.
const ANSI_SEQUENCE_RE = new RegExp(
    "(?:[\\u001B][\\]]|\\u009D)[\\s\\S]*?(?:\\u0007|\\u001B\\\\|\\u009C)|" +
        "[\\u001B\\u009B][[\\]()#;?]*(?:(?:(?:(?:;[-a-zA-Z\\d/#&.:=?%@~_]+)*|" +
        "[a-zA-Z\\d]+(?:;[-a-zA-Z\\d/#&.:=?%@~_]*)*)?\\u0007)|" +
        "(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-ntqry=><~]))",
    "g"
);

/**
 * Strips ANSI escape sequences and other terminal control bytes from `text`,
 * then converts bare `\n` to `\r\n` so multi-line text renders as separate
 * lines instead of a cursor staircase (xterm.js, like a real terminal,
 * treats `\n` as line-feed-only — it doesn't imply carriage return).
 */
export const sanitizeLogTextForTerminal = (text: string): string => {
    const withoutAnsi = text
        .replace(ANSI_SEQUENCE_RE, "")
        // Any stray control byte not part of a matched sequence above
        // (malformed/truncated escapes, bare ESC, BEL, CR, etc.) — \t and \n
        // are kept; \n is converted to \r\n next.
        // eslint-disable-next-line no-control-regex -- matching control bytes is the point
        .replace(/[\x00-\x08\x0b-\x1f\x7f]/g, "");
    return withoutAnsi.replace(/\n/g, "\r\n");
};

/** One log line as written into the shell terminal: tagged, sanitized, colored by level. */
export const formatLogLine = (tag: string, text: string, level?: "info" | "error" | "warn"): string => {
    const body = `[${tag}] ${sanitizeLogTextForTerminal(text)}`;
    if (level === "error") return `\x1b[31m${body}\x1b[0m`;
    if (level === "warn") return `\x1b[33m${body}\x1b[0m`;
    return `\x1b[90m${body}\x1b[0m`;
};

export interface ShellLogBridge {
    /** Log sink for every hook that takes a `LogFn`; only "system" lines reach the shell. */
    log: (tag: string, text: string, level?: "info" | "error" | "warn") => void;
    /** The shell terminal mounted: replay the backlog it hasn't seen, then write live. */
    onTermReady: (write: (text: string) => void) => void;
    /** The shell terminal unmounted. */
    onTermDispose: () => void;
    /** Stop writing to the terminal (its shell exited). */
    clearTermWrite: () => void;
}

export function useShellLogBridge(): ShellLogBridge {
    // Activity log — collects per-session diagnostic entries from launch
    // flow, subprocess lifecycle, slash commands, errors, etc. `log` is
    // passed down to every hook whose signature takes a `LogFn`, but only
    // "system"-tagged entries (bang-command output, `useAgentCommands.ts`'s
    // `dispatchBangCommand`; slash-command results, `commands/dispatch.ts`)
    // are genuinely user-initiated console-style interactions written into
    // the shell terminal (AgentShellSubblock's `onTermReady`) — everything
    // else (launch-flow status, auth prompts, CLI resolution, etc.) is
    // passive app-internal noise the shell should stay clean of. First cut
    // redirected every tag, which made the shell open with a wall of
    // "[cli] checking for claude...", "[auth] ..." etc. sitting above the
    // real prompt — reported live after removing the separate log panel.
    // `logLines` stays as a backlog (system-tagged entries only) so a bang
    // command's output logged while the drawer is closed still shows once
    // it reopens. `logFlushedCount` tracks how many of `logLines()` have
    // already been written into *some* terminal instance (live or
    // replayed) — every write, whether live or catch-up, advances it.
    // Without this, each drawer close/reopen replayed the entire backlog
    // again on top of whatever real PTY content the terminal (now durably)
    // restored (SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md).
    const { lines: logLines, append: appendLog } = useActivityLog();
    const [termWrite, setTermWrite] = createSignal<((text: string) => void) | null>(null);
    let logFlushedCount = 0;

    const log = (tag: string, text: string, level?: "info" | "error" | "warn") => {
        if (tag !== "system") return;
        appendLog(tag, text, level);
        const write = termWrite();
        if (write) {
            write(formatLogLine(tag, text, level));
            logFlushedCount = logLines().length;
        }
    };

    // Fired once per terminal mount (drawer open) — replays only the log
    // lines added since the last flush (whether that flush was this same
    // catch-up on a prior mount, or a live write while the drawer was open),
    // then keeps the write function around so `log` above writes live from
    // here on.
    const handleShellTermReady = (write: (text: string) => void) => {
        const all = logLines();
        for (let i = logFlushedCount; i < all.length; i++) {
            write(formatLogLine(all[i].tag, all[i].text, all[i].level));
        }
        logFlushedCount = all.length;
        setTermWrite(() => write);
    };
    const handleShellTermDispose = () => setTermWrite(null);

    return {
        log,
        onTermReady: handleShellTermReady,
        onTermDispose: handleShellTermDispose,
        clearTermWrite: () => setTermWrite(null),
    };
}
