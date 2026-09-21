// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show, type JSX } from "solid-js";

import { fullConfigAtom, settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const TERMINAL_SETTINGS = {
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
        description: "In agent composer: Shift+Enter inserts a newline instead of submitting",
        section: "terminal",
        keywords: ["newline", "multiline input", "composer keybinding", "term:shiftenternewline"],
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

// ── Section: Terminal ─────────────────────────────────────────────────────────

export function TerminalSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const termThemes = () => Object.entries((fullConfigAtom()?.termthemes as Record<string, any>) ?? {})
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));

    return (
        <div class="settings-section-body">
            <SettingRow
                id={TERMINAL_SETTINGS.fontSize.id}
                label={TERMINAL_SETTINGS.fontSize.label}
                description={TERMINAL_SETTINGS.fontSize.description}
                control={
                    <input
                        class="setting-number"
                        type="number" min={8} max={32}
                        value={(s()["term:fontsize"] as number) ?? 14}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 8 && v <= 32) set("term:fontsize", v);
                        }}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.fontFamily.id}
                label={TERMINAL_SETTINGS.fontFamily.label}
                description={TERMINAL_SETTINGS.fontFamily.description}
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["term:fontfamily"] as string) ?? ""}
                        placeholder="Hack, Consolas, monospace"
                        onBlur={(e) => set("term:fontfamily", e.currentTarget.value)}
                    />
                }
            />
            <Show when={termThemes().length > 0}>
                <SettingRow
                    id={TERMINAL_SETTINGS.terminalTheme.id}
                    label={TERMINAL_SETTINGS.terminalTheme.label}
                    control={
                        <select
                            class="setting-select"
                            value={(s()["term:theme"] as string) ?? ""}
                            onChange={(e) => set("term:theme", e.currentTarget.value || null)}
                        >
                            <option value="">Default</option>
                            <For each={termThemes()}>
                                {([key, theme]) => <option value={key}>{theme["display:name"] ?? key}</option>}
                            </For>
                        </select>
                    }
                />
            </Show>
            <SettingRow
                id={TERMINAL_SETTINGS.scrollback.id}
                label={TERMINAL_SETTINGS.scrollback.label}
                description={TERMINAL_SETTINGS.scrollback.description}
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1000} max={100000} step={1000}
                        value={(s()["term:scrollback"] as number) ?? 10000}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 1000 && v <= 100000) set("term:scrollback", v);
                        }}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.copyOnSelect.id}
                label={TERMINAL_SETTINGS.copyOnSelect.label}
                description={TERMINAL_SETTINGS.copyOnSelect.description}
                control={
                    <ToggleControl
                        checked={!!(s()["term:copyonselect"] as boolean)}
                        onChange={(v) => set("term:copyonselect", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.shiftEnterNewline.id}
                label={TERMINAL_SETTINGS.shiftEnterNewline.label}
                description={TERMINAL_SETTINGS.shiftEnterNewline.description}
                control={
                    <ToggleControl
                        checked={!!(s()["term:shiftenternewline"] as boolean)}
                        onChange={(v) => set("term:shiftenternewline", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.bracketedPaste.id}
                label={TERMINAL_SETTINGS.bracketedPaste.label}
                description={TERMINAL_SETTINGS.bracketedPaste.description}
                control={
                    <ToggleControl
                        checked={s()["term:allowbracketedpaste"] !== false}
                        onChange={(v) => set("term:allowbracketedpaste", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.terminalTransparency.id}
                label={TERMINAL_SETTINGS.terminalTransparency.label}
                description={TERMINAL_SETTINGS.terminalTransparency.description}
                control={
                    <SliderControl
                        min={0} max={1} step={0.05}
                        value={(s()["term:transparency"] as number) ?? 0.5}
                        onChange={(v) => set("term:transparency", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.scrollSensitivity.id}
                label={TERMINAL_SETTINGS.scrollSensitivity.label}
                description={TERMINAL_SETTINGS.scrollSensitivity.description}
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={0.1} max={10} step={0.1}
                        value={(s()["term:scrollsensitivity"] as number) ?? 1}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 0.1 && v <= 10) set("term:scrollsensitivity", v);
                        }}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.cpuMemBadge.id}
                label={TERMINAL_SETTINGS.cpuMemBadge.label}
                description={TERMINAL_SETTINGS.cpuMemBadge.description}
                control={
                    <ToggleControl
                        checked={s()["term:showstatsbadge"] !== false}
                        onChange={(v) => set("term:showstatsbadge", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.predictiveEcho.id}
                label={TERMINAL_SETTINGS.predictiveEcho.label}
                description={TERMINAL_SETTINGS.predictiveEcho.description}
                control={
                    <ToggleControl
                        checked={!!(s()["term:predictiveecho"] as boolean)}
                        onChange={(v) => set("term:predictiveecho", v)}
                    />
                }
            />
            <Show when={!!(s()["term:predictiveecho"] as boolean)}>
                <SettingRow
                    id={TERMINAL_SETTINGS.predictiveEchoThreshold.id}
                    indent
                    label={TERMINAL_SETTINGS.predictiveEchoThreshold.label}
                    description={TERMINAL_SETTINGS.predictiveEchoThreshold.description}
                    control={
                        <input
                            class="setting-number setting-number--wide"
                            type="number" min={0}
                            value={(s()["term:predictiveecho:thresholdms"] as number) ?? 100}
                            onBlur={(e) => {
                                const v = parseFloat(e.currentTarget.value);
                                if (!isNaN(v) && v >= 0) set("term:predictiveecho:thresholdms", v);
                            }}
                        />
                    }
                />
            </Show>
            <SettingRow
                id={TERMINAL_SETTINGS.agentMaxRuntime.id}
                label={TERMINAL_SETTINGS.agentMaxRuntime.label}
                description={TERMINAL_SETTINGS.agentMaxRuntime.description}
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={0} step={0.5}
                        value={(s()["term:agentmaxruntimehours"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 0) set("term:agentmaxruntimehours", v);
                        }}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.agentIdleTimeout.id}
                label={TERMINAL_SETTINGS.agentIdleTimeout.label}
                description={TERMINAL_SETTINGS.agentIdleTimeout.description}
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={0}
                        value={(s()["term:agentidletimeoutmins"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 0) set("term:agentidletimeoutmins", v);
                        }}
                    />
                }
            />
        </div>
    );
}
