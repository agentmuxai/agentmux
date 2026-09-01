// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show, type JSX } from "solid-js";

import { fullConfigAtom, settingsAtom } from "@/app/store/global";
import { set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";

// ── Section: Terminal ─────────────────────────────────────────────────────────

export function TerminalSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const termThemes = () => Object.entries((fullConfigAtom()?.termthemes as Record<string, any>) ?? {})
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Font size"
                description="Terminal font size in pixels (8–32)"
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
                label="Font family"
                description="Comma-separated font fallback list"
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["term:fontfamily"] as string) ?? ""}
                        placeholder="JetBrains Mono, monospace"
                        onBlur={(e) => set("term:fontfamily", e.currentTarget.value)}
                    />
                }
            />
            <Show when={termThemes().length > 0}>
                <SettingRow
                    label="Terminal color theme"
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
                label="Scrollback lines"
                description="Number of lines kept in terminal scrollback (1000–100000)"
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
                label="Copy on select"
                description="Automatically copy selected text to clipboard"
                control={
                    <ToggleControl
                        checked={!!(s()["term:copyonselect"] as boolean)}
                        onChange={(v) => set("term:copyonselect", v)}
                    />
                }
            />
            <SettingRow
                label="Shift+Enter → new line"
                description="In agent composer: Shift+Enter inserts a newline instead of submitting"
                control={
                    <ToggleControl
                        checked={!!(s()["term:shiftenternewline"] as boolean)}
                        onChange={(v) => set("term:shiftenternewline", v)}
                    />
                }
            />
            <SettingRow
                label="Bracketed paste"
                description="Allow programs to detect pasted text vs. typed text"
                control={
                    <ToggleControl
                        checked={s()["term:allowbracketedpaste"] !== false}
                        onChange={(v) => set("term:allowbracketedpaste", v)}
                    />
                }
            />
            <SettingRow
                label="Terminal transparency"
                description="Terminal background transparency (0 = opaque, 1 = fully transparent)"
                control={
                    <SliderControl
                        min={0} max={1} step={0.05}
                        value={(s()["term:transparency"] as number) ?? 0.5}
                        onChange={(v) => set("term:transparency", v)}
                    />
                }
            />
            <SettingRow
                label="Scroll sensitivity"
                description="Scroll wheel speed multiplier for terminal panes (0.1–10, default 1). Independent of the OS scroll-speed setting."
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
                label="Predictive echo"
                description="Show local predictive echo of typed characters while waiting on a slow/remote shell"
                control={
                    <ToggleControl
                        checked={!!(s()["term:predictiveecho"] as boolean)}
                        onChange={(v) => set("term:predictiveecho", v)}
                    />
                }
            />
            <Show when={!!(s()["term:predictiveecho"] as boolean)}>
                <SettingRow
                    indent
                    label="Predictive echo threshold"
                    description="Round-trip latency (ms) above which predictive echo kicks in"
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
                label="Agent max runtime"
                description="Hours before the watchdog kills a long-running agent pane. 0 = no limit."
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
                label="Agent idle timeout"
                description="Minutes of PTY silence before the watchdog kills an idle agent pane. 0 = no limit."
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
