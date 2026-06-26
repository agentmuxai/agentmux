// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, Match, Show, Switch, type JSX } from "solid-js";

import { fullConfigAtom, settingsAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { invokeCommand } from "@/app/platform/ipc";
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import type { SettingsSection, SettingsViewModel } from "./settings-model";
import "./settings.scss";

// ── Helpers ───────────────────────────────────────────────────────────────────

function set(key: string, value: unknown): void {
    void RpcApi.SetConfigCommand(TabRpcClient, { [key]: value } as any);
}

function makeDebounce(ms: number) {
    let timer: ReturnType<typeof setTimeout> | null = null;
    return (fn: () => void) => {
        if (timer != null) clearTimeout(timer);
        timer = setTimeout(() => { timer = null; fn(); }, ms);
    };
}

const sliderDebounce = makeDebounce(180);

// ── SettingRow primitive ──────────────────────────────────────────────────────

function SettingRow(p: { label: string; description?: string; control: JSX.Element; indent?: boolean }): JSX.Element {
    return (
        <div class="setting-row" classList={{ "setting-row--indent": p.indent }}>
            <div class="setting-row-label">
                <span class="setting-row-name">{p.label}</span>
                <Show when={p.description}>
                    <span class="setting-row-desc">{p.description}</span>
                </Show>
            </div>
            <div class="setting-row-control">{p.control}</div>
        </div>
    );
}

function ToggleControl(p: { checked: boolean; onChange: (v: boolean) => void }): JSX.Element {
    return (
        <button
            type="button"
            role="switch"
            aria-checked={p.checked}
            class="setting-toggle"
            classList={{ "setting-toggle--on": p.checked }}
            onClick={() => p.onChange(!p.checked)}
        >
            <span class="setting-toggle-thumb" />
        </button>
    );
}

function SliderControl(p: { min: number; max: number; step: number; value: number; onChange: (v: number) => void }): JSX.Element {
    const [local, setLocal] = createSignal(p.value);
    return (
        <div class="setting-slider">
            <input
                type="range"
                min={p.min} max={p.max} step={p.step}
                value={local()}
                onInput={(e) => {
                    const v = parseFloat(e.currentTarget.value);
                    setLocal(v);
                    sliderDebounce(() => p.onChange(v));
                }}
            />
            <span class="setting-slider-val">{Math.round(local() * 100) / 100}</span>
        </div>
    );
}

// ── Config error banner ───────────────────────────────────────────────────────

function ConfigErrorsBanner(): JSX.Element {
    const errors = () => fullConfigAtom()?.configerrors ?? [];
    const openRaw = async () => {
        const path = await invokeCommand<string>("ensure_settings_file");
        await invokeCommand("open_in_editor", { path });
    };
    return (
        <Show when={errors().length > 0}>
            <div class="settings-config-errors">
                <For each={errors()}>
                    {(e: any) => (
                        <div class="settings-config-error">
                            <i class="fa-solid fa-circle-exclamation" />
                            {" "}{e.err}
                            <Show when={e.file}>
                                {" "}<span class="mono">{e.file}</span>
                            </Show>
                        </div>
                    )}
                </For>
                <button class="settings-config-error-fix" onClick={() => void openRaw()}>
                    Fix in editor
                </button>
            </div>
        </Show>
    );
}

// ── Section: Appearance ───────────────────────────────────────────────────────

function AppearanceSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const transparent = () => !!(s()["window:transparent"] as boolean);

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Theme"
                description="UI color theme for all windows"
                control={
                    <select
                        class="setting-select"
                        value={(s()["window:theme"] as string) ?? "default"}
                        onChange={(e) => set("window:theme", e.currentTarget.value)}
                    >
                        <For each={THEME_OPTIONS}>
                            {(t) => <option value={t.id}>{t.label}</option>}
                        </For>
                    </select>
                }
            />
            <SettingRow
                label="Window transparency"
                description="Enable background transparency and blur"
                control={
                    <ToggleControl
                        checked={transparent()}
                        onChange={(v) => set("window:transparent", v)}
                    />
                }
            />
            <Show when={transparent()}>
                <SettingRow
                    indent
                    label="Opacity"
                    description="Window background opacity (35–100%)"
                    control={
                        <SliderControl
                            min={0.35} max={1} step={0.05}
                            value={(s()["window:opacity"] as number) ?? 1}
                            onChange={(v) => set("window:opacity", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Background blur"
                    description="Blur the content behind the window"
                    control={
                        <ToggleControl
                            checked={!!(s()["window:blur"] as boolean)}
                            onChange={(v) => set("window:blur", v)}
                        />
                    }
                />
            </Show>
            <SettingRow
                label="Pane gap size"
                description="Pixels between tiled panes (0–20)"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0} max={20}
                        value={(s()["window:tilegapsize"] as number) ?? 4}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0 && v <= 20) set("window:tilegapsize", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Reduce motion"
                description="Disable CSS animations and transitions"
                control={
                    <ToggleControl
                        checked={!!(s()["window:reducedmotion"] as boolean)}
                        onChange={(v) => set("window:reducedmotion", v)}
                    />
                }
            />
        </div>
    );
}

// ── Section: Terminal ─────────────────────────────────────────────────────────

function TerminalSection(): JSX.Element {
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
                        value={(s()["term:transparency"] as number) ?? 0}
                        onChange={(v) => set("term:transparency", v)}
                    />
                }
            />
        </div>
    );
}

// ── Section stubs (Phases 3–4) ────────────────────────────────────────────────

function StubSection(p: { label: string }): JSX.Element {
    return (
        <div class="settings-section-body settings-section-stub">
            <i class="fa-solid fa-screwdriver-wrench" />
            <span>{p.label} settings coming soon.</span>
        </div>
    );
}

// ── Rail ──────────────────────────────────────────────────────────────────────

const RAIL: { id: SettingsSection; label: string; icon: string }[] = [
    { id: "appearance", label: "Appearance",    icon: "palette" },
    { id: "terminal",   label: "Terminal",      icon: "square-terminal" },
    { id: "agent",      label: "Agent",         icon: "sparkles" },
    { id: "sounds",     label: "Sounds",        icon: "volume-high" },
    { id: "network",    label: "Network",       icon: "wifi" },
    { id: "files",      label: "Files",         icon: "folder-open" },
    { id: "advanced",   label: "Advanced",      icon: "sliders" },
];

// ── Main view ─────────────────────────────────────────────────────────────────

export function SettingsView(props: ViewComponentProps<SettingsViewModel>): JSX.Element {
    const section = () => props.model.activeSection();
    const setSection = (s: SettingsSection) => props.model.setSection(s);

    const openRaw = async () => {
        const path = await invokeCommand<string>("ensure_settings_file");
        await invokeCommand("open_in_editor", { path });
    };

    return (
        <div class="settings-view">
            <nav class="settings-rail" aria-label="Settings section">
                <For each={RAIL}>
                    {(item) => (
                        <button
                            type="button"
                            class="settings-rail-item"
                            classList={{ "is-active": section() === item.id }}
                            aria-pressed={section() === item.id}
                            onClick={() => setSection(item.id)}
                        >
                            <i class={`fa-solid fa-${item.icon}`} aria-hidden="true" />
                            <span>{item.label}</span>
                        </button>
                    )}
                </For>
            </nav>
            <div class="settings-body">
                <ConfigErrorsBanner />
                <Switch>
                    <Match when={section() === "appearance"}>
                        <AppearanceSection />
                    </Match>
                    <Match when={section() === "terminal"}>
                        <TerminalSection />
                    </Match>
                    <Match when={section() === "agent"}>
                        <StubSection label="Agent" />
                    </Match>
                    <Match when={section() === "sounds"}>
                        <StubSection label="Sounds & Notifications" />
                    </Match>
                    <Match when={section() === "network"}>
                        <StubSection label="Network" />
                    </Match>
                    <Match when={section() === "files"}>
                        <StubSection label="Files & Drag-Drop" />
                    </Match>
                    <Match when={section() === "advanced"}>
                        <StubSection label="Advanced" />
                    </Match>
                </Switch>
                <footer class="settings-footer">
                    <button class="settings-footer-btn" onClick={() => void openRaw()}>
                        <i class="fa-solid fa-file-code" /> Open raw settings.json
                    </button>
                </footer>
            </div>
        </div>
    );
}
