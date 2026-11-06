// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SettingsIndexEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const TERMINAL_SETTINGS = {
    defaultShell: {
        id: "terminal.default_shell",
        label: "Default shell",
        description: "The shell new local terminals open with. Agents' shells and SSH or WSL terminals keep their own.",
        section: "terminal",
        keywords: ["shell", "default shell", "pwsh", "powershell", "bash", "zsh", "fish", "git bash", "cmd", "nushell", "term:localshellpath"],
    },
    fontSize: {
        id: "terminal.font_size",
        label: "Font size",
        description: "Terminal font size in pixels (8–32)",
        section: "terminal",
        keywords: ["text size", "zoom", "make text bigger", "font scale", "term:fontsize"],
    },
    fontFamily: {
        id: "terminal.font_family",
        label: "Font family",
        description: "Comma-separated font fallback list",
        section: "terminal",
        keywords: ["typeface", "monospace font", "term:fontfamily"],
    },
    terminalTheme: {
        id: "terminal.theme",
        label: "Terminal color theme",
        section: "terminal",
        keywords: ["terminal colors", "ansi colors", "color scheme", "term:theme"],
    },
    scrollback: {
        id: "terminal.scrollback",
        label: "Scrollback lines",
        description: "Number of lines kept in terminal scrollback (1000–100000)",
        section: "terminal",
        keywords: ["scroll history", "buffer size", "history lines", "term:scrollback"],
    },
    copyOnSelect: {
        id: "terminal.copy_on_select",
        label: "Copy on select",
        description: "Automatically copy selected text to clipboard",
        section: "terminal",
        keywords: ["auto copy", "clipboard", "select to copy", "term:copyonselect"],
    },
    shiftEnterNewline: {
        id: "terminal.shift_enter_newline",
        label: "Shift+Enter → new line",
        description: "In a terminal: Shift+Enter sends a newline instead of running the line",
        section: "terminal",
        keywords: ["newline", "multiline input", "terminal keybinding", "term:shiftenternewline"],
    },
    durableSsh: {
        id: "terminal.durable_ssh",
        label: "Keep SSH sessions alive",
        description:
            "Durable SSH terminals keep their shell running on the host (in AgentMux's small helper there), so a dropped link, sleep or a restart reconnects instead of ending it. On a host without the helper, the first terminal asks before installing it, and is a plain terminal if it isn't installed. Connectors → Remotes can set it per host.",
        section: "terminal",
        keywords: ["durable", "ssh", "reconnect", "remote", "session", "term:durable"],
    },
    helperInstall: {
        id: "terminal.helper_install",
        label: "Install the helper on new hosts",
        description:
            "Durable terminals and file browsing on an SSH host need AgentMux's small helper there. Ask before installing it, install it without asking, or never install it. Each host can override this in Connectors → Remotes, and an agent's install is always asked about.",
        section: "terminal",
        keywords: ["helper", "install", "ssh", "remote", "agentmux-remote", "conn:helper"],
    },
    bracketedPaste: {
        id: "terminal.bracketed_paste",
        label: "Bracketed paste",
        description: "Allow programs to detect pasted text vs. typed text",
        section: "terminal",
        keywords: ["paste mode", "term:allowbracketedpaste"],
    },
    terminalTransparency: {
        id: "terminal.transparency",
        label: "Terminal transparency",
        description: "Terminal background transparency (0 = opaque, 1 = fully transparent)",
        section: "terminal",
        keywords: ["transparent terminal", "opacity", "term:transparency"],
    },
    scrollSensitivity: {
        id: "terminal.scroll_sensitivity",
        label: "Scroll sensitivity",
        description: "Scroll wheel speed multiplier for terminal panes (0.1–10, default 1). Independent of the OS scroll-speed setting.",
        section: "terminal",
        keywords: ["scroll speed", "mouse wheel speed", "term:scrollsensitivity"],
    },
    cpuMemBadge: {
        id: "terminal.cpu_mem_badge",
        label: "Show CPU/mem badge",
        description: "Show a live CPU%/memory usage badge in the top-right corner of terminal panes",
        section: "terminal",
        keywords: ["cpu usage", "memory usage", "resource badge", "term:showstatsbadge"],
    },
    predictiveEcho: {
        id: "terminal.predictive_echo",
        label: "Predictive echo",
        description: "Show local predictive echo of typed characters while waiting on a slow/remote shell",
        section: "terminal",
        keywords: ["local echo", "typing lag", "ssh latency", "term:predictiveecho"],
    },
    predictiveEchoThreshold: {
        id: "terminal.predictive_echo_threshold",
        label: "Predictive echo threshold",
        description: "Round-trip latency (ms) above which predictive echo kicks in",
        section: "terminal",
        keywords: ["echo latency", "round trip time", "term:predictiveecho:thresholdms"],
    },
    agentMaxRuntime: {
        id: "terminal.agent_max_runtime",
        label: "Agent max runtime",
        description: "Hours before the watchdog kills a long-running agent pane. 0 = no limit.",
        section: "terminal",
        keywords: ["max runtime", "runtime limit", "watchdog", "term:agentmaxruntimehours"],
    },
    agentIdleTimeout: {
        id: "terminal.agent_idle_timeout",
        label: "Agent idle timeout",
        description: "Minutes of PTY silence before the watchdog kills an idle agent pane. 0 = no limit.",
        section: "terminal",
        keywords: ["idle timeout", "auto-lock", "sleep", "inactivity timer", "term:agentidletimeoutmins"],
    },
} satisfies Record<string, SettingsIndexEntry>;
