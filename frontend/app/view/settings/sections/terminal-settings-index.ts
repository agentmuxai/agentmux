// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { settingEntry, type SettingsIndexEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const TERMINAL_SETTINGS = {
    defaultShell: settingEntry({
        id: "terminal.default_shell",
        labelKey: "settings.terminal.defaultShell.label",
        descriptionKey: "settings.terminal.defaultShell.description",
        section: "terminal",
        keywords: ["shell", "default shell", "pwsh", "powershell", "bash", "zsh", "fish", "git bash", "cmd", "nushell", "term:localshellpath"],
    }),
    fontSize: settingEntry({
        id: "terminal.font_size",
        labelKey: "settings.terminal.fontSize.label",
        descriptionKey: "settings.terminal.fontSize.description",
        section: "terminal",
        keywords: ["text size", "zoom", "make text bigger", "font scale", "term:fontsize"],
    }),
    fontFamily: settingEntry({
        id: "terminal.font_family",
        labelKey: "settings.terminal.fontFamily.label",
        descriptionKey: "settings.terminal.fontFamily.description",
        section: "terminal",
        keywords: ["typeface", "monospace font", "term:fontfamily"],
    }),
    terminalTheme: settingEntry({
        id: "terminal.theme",
        labelKey: "settings.terminal.terminalTheme.label",
        section: "terminal",
        keywords: ["terminal colors", "ansi colors", "color scheme", "term:theme"],
    }),
    scrollback: settingEntry({
        id: "terminal.scrollback",
        labelKey: "settings.terminal.scrollback.label",
        descriptionKey: "settings.terminal.scrollback.description",
        section: "terminal",
        keywords: ["scroll history", "buffer size", "history lines", "term:scrollback"],
    }),
    copyOnSelect: settingEntry({
        id: "terminal.copy_on_select",
        labelKey: "settings.terminal.copyOnSelect.label",
        descriptionKey: "settings.terminal.copyOnSelect.description",
        section: "terminal",
        keywords: ["auto copy", "clipboard", "select to copy", "term:copyonselect"],
    }),
    shiftEnterNewline: settingEntry({
        id: "terminal.shift_enter_newline",
        labelKey: "settings.terminal.shiftEnterNewline.label",
        descriptionKey: "settings.terminal.shiftEnterNewline.description",
        section: "terminal",
        keywords: ["newline", "multiline input", "terminal keybinding", "term:shiftenternewline"],
    }),
    durableSsh: settingEntry({
        id: "terminal.durable_ssh",
        labelKey: "settings.terminal.durableSsh.label",
        descriptionKey: "settings.terminal.durableSsh.description",
        section: "terminal",
        keywords: ["durable", "ssh", "reconnect", "remote", "session", "term:durable"],
    }),
    helperInstall: settingEntry({
        id: "terminal.helper_install",
        labelKey: "settings.terminal.helperInstall.label",
        descriptionKey: "settings.terminal.helperInstall.description",
        section: "terminal",
        keywords: ["helper", "install", "ssh", "remote", "agentmux-remote", "conn:helper"],
    }),
    bracketedPaste: settingEntry({
        id: "terminal.bracketed_paste",
        labelKey: "settings.terminal.bracketedPaste.label",
        descriptionKey: "settings.terminal.bracketedPaste.description",
        section: "terminal",
        keywords: ["paste mode", "term:allowbracketedpaste"],
    }),
    terminalTransparency: settingEntry({
        id: "terminal.transparency",
        labelKey: "settings.terminal.terminalTransparency.label",
        descriptionKey: "settings.terminal.terminalTransparency.description",
        section: "terminal",
        keywords: ["transparent terminal", "opacity", "term:transparency"],
    }),
    scrollSensitivity: settingEntry({
        id: "terminal.scroll_sensitivity",
        labelKey: "settings.terminal.scrollSensitivity.label",
        descriptionKey: "settings.terminal.scrollSensitivity.description",
        section: "terminal",
        keywords: ["scroll speed", "mouse wheel speed", "term:scrollsensitivity"],
    }),
    cpuMemBadge: settingEntry({
        id: "terminal.cpu_mem_badge",
        labelKey: "settings.terminal.cpuMemBadge.label",
        descriptionKey: "settings.terminal.cpuMemBadge.description",
        section: "terminal",
        keywords: ["cpu usage", "memory usage", "resource badge", "term:showstatsbadge"],
    }),
    predictiveEcho: settingEntry({
        id: "terminal.predictive_echo",
        labelKey: "settings.terminal.predictiveEcho.label",
        descriptionKey: "settings.terminal.predictiveEcho.description",
        section: "terminal",
        keywords: ["local echo", "typing lag", "ssh latency", "term:predictiveecho"],
    }),
    predictiveEchoThreshold: settingEntry({
        id: "terminal.predictive_echo_threshold",
        labelKey: "settings.terminal.predictiveEchoThreshold.label",
        descriptionKey: "settings.terminal.predictiveEchoThreshold.description",
        section: "terminal",
        keywords: ["echo latency", "round trip time", "term:predictiveecho:thresholdms"],
    }),
    agentMaxRuntime: settingEntry({
        id: "terminal.agent_max_runtime",
        labelKey: "settings.terminal.agentMaxRuntime.label",
        descriptionKey: "settings.terminal.agentMaxRuntime.description",
        section: "terminal",
        keywords: ["max runtime", "runtime limit", "watchdog", "term:agentmaxruntimehours"],
    }),
    agentIdleTimeout: settingEntry({
        id: "terminal.agent_idle_timeout",
        labelKey: "settings.terminal.agentIdleTimeout.label",
        descriptionKey: "settings.terminal.agentIdleTimeout.description",
        section: "terminal",
        keywords: ["idle timeout", "auto-lock", "sleep", "inactivity timer", "term:agentidletimeoutmins"],
    }),
} satisfies Record<string, SettingsIndexEntry>;
