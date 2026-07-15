// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, Match, onCleanup, Show, Switch, type JSX } from "solid-js";

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

// ── SettingRow primitive ──────────────────────────────────────────────────────

function SettingRow(p: { label: string; description?: string; control: JSX.Element; indent?: boolean; stacked?: boolean }): JSX.Element {
    return (
        <div class="setting-row" classList={{ "setting-row--indent": p.indent, "setting-row--stacked": p.stacked }}>
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

function SectionHeader(p: { label: string }): JSX.Element {
    return <div class="settings-subheader">{p.label}</div>;
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
    createEffect(() => setLocal(p.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (timer != null) clearTimeout(timer); });
    return (
        <div class="setting-slider">
            <input
                type="range"
                min={p.min} max={p.max} step={p.step}
                value={local()}
                onInput={(e) => {
                    const v = parseFloat(e.currentTarget.value);
                    setLocal(v);
                    if (timer != null) clearTimeout(timer);
                    timer = setTimeout(() => { timer = null; p.onChange(v); }, 180);
                }}
            />
            <span class="setting-slider-val">{Math.round(local() * 100) / 100}</span>
        </div>
    );
}

function KeyValueEditor(p: { value: Record<string, string>; onChange: (v: Record<string, string>) => void }): JSX.Element {
    const keys = () => Object.keys(p.value ?? {});
    const [newKey, setNewKey] = createSignal("");
    const [newVal, setNewVal] = createSignal("");

    const updateEntry = (key: string, val: string) => p.onChange({ ...p.value, [key]: val });
    const removeEntry = (key: string) => {
        const next = { ...p.value };
        delete next[key];
        p.onChange(next);
    };
    const addEntry = () => {
        const k = newKey().trim();
        if (!k) return;
        p.onChange({ ...p.value, [k]: newVal() });
        setNewKey("");
        setNewVal("");
    };

    return (
        <div class="setting-kv-editor">
            <For each={keys()}>
                {(k) => (
                    <div class="setting-kv-row">
                        <input class="setting-text setting-kv-key" type="text" value={k} disabled />
                        <input
                            class="setting-text setting-kv-val"
                            type="text"
                            value={p.value[k] ?? ""}
                            onBlur={(e) => updateEntry(k, e.currentTarget.value)}
                        />
                        <button type="button" class="setting-kv-remove" onClick={() => removeEntry(k)}>
                            <i class="fa-solid fa-xmark" />
                        </button>
                    </div>
                )}
            </For>
            <div class="setting-kv-row setting-kv-row--new">
                <input
                    class="setting-text setting-kv-key"
                    type="text"
                    placeholder="KEY"
                    value={newKey()}
                    onInput={(e) => setNewKey(e.currentTarget.value)}
                />
                <input
                    class="setting-text setting-kv-val"
                    type="text"
                    placeholder="value"
                    value={newVal()}
                    onInput={(e) => setNewVal(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") addEntry(); }}
                />
                <button type="button" class="setting-kv-remove setting-kv-add" onClick={addEntry}>
                    <i class="fa-solid fa-plus" />
                </button>
            </div>
        </div>
    );
}

function StringArrayEditor(p: { value: string[]; onChange: (v: string[]) => void }): JSX.Element {
    const items = () => p.value ?? [];
    const [draft, setDraft] = createSignal("");

    const updateItem = (i: number, val: string) => {
        const next = items().slice();
        next[i] = val;
        p.onChange(next);
    };
    const removeItem = (i: number) => {
        const next = items().slice();
        next.splice(i, 1);
        p.onChange(next);
    };
    const addItem = () => {
        const v = draft().trim();
        if (!v) return;
        p.onChange([...items(), v]);
        setDraft("");
    };

    return (
        <div class="setting-kv-editor">
            <For each={items()}>
                {(item, i) => (
                    <div class="setting-kv-row">
                        <input
                            class="setting-text setting-kv-val"
                            type="text"
                            value={item}
                            onBlur={(e) => updateItem(i(), e.currentTarget.value)}
                        />
                        <button type="button" class="setting-kv-remove" onClick={() => removeItem(i())}>
                            <i class="fa-solid fa-xmark" />
                        </button>
                    </div>
                )}
            </For>
            <div class="setting-kv-row setting-kv-row--new">
                <input
                    class="setting-text setting-kv-val"
                    type="text"
                    placeholder="--flag value"
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") addItem(); }}
                />
                <button type="button" class="setting-kv-remove setting-kv-add" onClick={addItem}>
                    <i class="fa-solid fa-plus" />
                </button>
            </div>
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
            <SettingRow
                label="Background color"
                description="Custom background color override (hex). Leave blank to use the theme default."
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["window:bgcolor"] as string) ?? ""}
                        placeholder="#1a1a1a"
                        onBlur={(e) => set("window:bgcolor", e.currentTarget.value || null)}
                    />
                }
            />
            <SectionHeader label="Pane hover-magnify" />
            <SettingRow
                label="Magnified opacity"
                description="Background opacity of a pane while magnified (0–1)"
                control={
                    <SliderControl
                        min={0} max={1} step={0.05}
                        value={(s()["window:magnifiedblockopacity"] as number) ?? 1}
                        onChange={(v) => set("window:magnifiedblockopacity", v)}
                    />
                }
            />
            <SettingRow
                label="Magnified size"
                description="Scale factor applied to a pane while magnified"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1} step={0.1}
                        value={(s()["window:magnifiedblocksize"] as number) ?? 1.5}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 1) set("window:magnifiedblocksize", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Magnified blur (primary)"
                description="Backdrop blur, in pixels, applied to the magnified pane itself"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0}
                        value={(s()["window:magnifiedblockblurprimarypx"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0) set("window:magnifiedblockblurprimarypx", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Magnified blur (secondary)"
                description="Backdrop blur, in pixels, applied to the other panes behind it"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0}
                        value={(s()["window:magnifiedblockblursecondarypx"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0) set("window:magnifiedblockblursecondarypx", v);
                        }}
                    />
                }
            />
        </div>
    );
}

// ── Section: Window & Panes ───────────────────────────────────────────────────

function WindowPanesSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Show block IDs"
                description="Show each pane's internal block ID in its header (debugging aid)"
                control={
                    <ToggleControl
                        checked={!!(s()["blockheader:showblockids"] as boolean)}
                        onChange={(v) => set("blockheader:showblockids", v)}
                    />
                }
            />
            <SettingRow
                label="Default new block"
                description="View type opened by default for new tabs/panes (e.g. term, agent)"
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["app:defaultnewblock"] as string) ?? ""}
                        placeholder="term"
                        onBlur={(e) => set("app:defaultnewblock", e.currentTarget.value || null)}
                    />
                }
            />
            <SettingRow
                label="Show pane number overlay"
                description="Show numbered overlays for quick pane-jump shortcuts"
                control={
                    <ToggleControl
                        checked={!!(s()["app:showoverlayblocknums"] as boolean)}
                        onChange={(v) => set("app:showoverlayblocknums", v)}
                    />
                }
            />
            <SettingRow
                label="Skip tab close confirmation"
                description="Don't prompt for confirmation when closing a tab"
                control={
                    <ToggleControl
                        checked={!!(s()["tab:skipcloseconfirm"] as boolean)}
                        onChange={(v) => set("tab:skipcloseconfirm", v)}
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
                        value={(s()["term:transparency"] as number) ?? 0.5}
                        onChange={(v) => set("term:transparency", v)}
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
        </div>
    );
}

// ── Section: Sounds & Notifications ───────────────────────────────────────────

function SoundsSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const soundsEnabled = () => s()["notify:sounds:enabled"] !== false;
    const toolTonesEnabled = () => s()["notify:tooltones:enabled"] !== false;

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Notification sounds"
                description="Master enable for notification sounds"
                control={<ToggleControl checked={soundsEnabled()} onChange={(v) => set("notify:sounds:enabled", v)} />}
            />
            <Show when={soundsEnabled()}>
                <SettingRow
                    indent
                    label="Volume"
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:sounds:volume"] as number) ?? 0.6}
                            onChange={(v) => set("notify:sounds:volume", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Suppress when focused"
                    description="Don't play a pane's sound when it's already focused and visible"
                    control={
                        <ToggleControl
                            checked={s()["notify:sounds:suppresswhenfocused"] !== false}
                            onChange={(v) => set("notify:sounds:suppresswhenfocused", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn complete"
                    description="Play a sound when an agent turn completes normally"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.complete"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.complete", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn error"
                    description="Play a sound when an agent turn ends with an error"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.error"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.error", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn interrupted"
                    description="Play a sound when an agent turn is stopped or interrupted"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.interrupted"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.interrupted", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Message accepted"
                    description="Play a sound when a queued pending message is accepted"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.message.accepted"] !== false}
                            onChange={(v) => set("notify:sound:agent.message.accepted", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Message rejected"
                    description="Play a sound when a queued pending message is rejected"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.message.rejected"] !== false}
                            onChange={(v) => set("notify:sound:agent.message.rejected", v)}
                        />
                    }
                />
            </Show>
            <SectionHeader label="Tool-call tones" />
            <SettingRow
                label="Tool-call tones"
                description="Play a subliminal synth tone for every agent tool call"
                control={<ToggleControl checked={toolTonesEnabled()} onChange={(v) => set("notify:tooltones:enabled", v)} />}
            />
            <Show when={toolTonesEnabled()}>
                <SettingRow
                    indent
                    label="Volume"
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:tooltones:volume"] as number) ?? 0.15}
                            onChange={(v) => set("notify:tooltones:volume", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Scope"
                    description="Which panes play tool-call tones"
                    control={
                        <select
                            class="setting-select"
                            value={(s()["notify:tooltones:scope"] as string) ?? "all"}
                            onChange={(e) => set("notify:tooltones:scope", e.currentTarget.value)}
                        >
                            <option value="all">All panes</option>
                            <option value="focused">Focused pane only</option>
                        </select>
                    }
                />
            </Show>
        </div>
    );
}

// ── Section: Advanced ─────────────────────────────────────────────────────────

function AdvancedSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SectionHeader label="Terminal (power user)" />
            <SettingRow
                label="Disable WebGL rendering"
                description="Fall back to canvas-based terminal rendering (restart required)"
                control={
                    <ToggleControl
                        checked={!!(s()["term:disablewebgl"] as boolean)}
                        onChange={(v) => set("term:disablewebgl", v)}
                    />
                }
            />
            <SettingRow
                label="Local shell path"
                description="Override the executable used for local terminal panes (restart required)"
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["term:localshellpath"] as string) ?? ""}
                        placeholder="/bin/zsh"
                        onBlur={(e) => set("term:localshellpath", e.currentTarget.value || null)}
                    />
                }
            />
            <SettingRow
                stacked
                label="Local shell arguments"
                description="Extra arguments passed to the local shell on launch"
                control={
                    <StringArrayEditor
                        value={(s()["term:localshellopts"] as string[]) ?? []}
                        onChange={(v) => set("term:localshellopts", v)}
                    />
                }
            />
            <SectionHeader label="Widgets" />
            <SettingRow
                label="Icon-only widget labels"
                description="Force the widget bar to show icons without text labels"
                control={
                    <ToggleControl
                        checked={!!(s()["widget:icononly"] as boolean)}
                        onChange={(v) => set("widget:icononly", v)}
                    />
                }
            />
            <SectionHeader label="Sysinfo widget" />
            <SettingRow
                label="Sample interval"
                description="Seconds between sysinfo widget samples"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1}
                        value={(s()["telemetry:interval"] as number) ?? 1}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 1) set("telemetry:interval", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="History length"
                description="Number of sysinfo widget samples retained (30–1024)"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={30} max={1024}
                        value={(s()["telemetry:numpoints"] as number) ?? 120}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 30 && v <= 1024) set("telemetry:numpoints", v);
                        }}
                    />
                }
            />
            <SectionHeader label="Environment" />
            <SettingRow
                stacked
                label="Global environment variables"
                description="Environment variables injected into every shell"
                control={
                    <KeyValueEditor
                        value={(s()["cmd:env"] as Record<string, string>) ?? {}}
                        onChange={(v) => set("cmd:env", v)}
                    />
                }
            />
        </div>
    );
}

// ── Rail ──────────────────────────────────────────────────────────────────────

const RAIL: { id: SettingsSection; label: string; icon: string }[] = [
    { id: "appearance", label: "Appearance",     icon: "palette" },
    { id: "window",     label: "Window & Panes", icon: "table-cells" },
    { id: "terminal",   label: "Terminal",       icon: "square-terminal" },
    { id: "sounds",     label: "Sounds",         icon: "volume-high" },
    { id: "advanced",   label: "Advanced",       icon: "sliders" },
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
        <div class="settings-view-container">
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
                    <Match when={section() === "window"}>
                        <WindowPanesSection />
                    </Match>
                    <Match when={section() === "terminal"}>
                        <TerminalSection />
                    </Match>
                    <Match when={section() === "sounds"}>
                        <SoundsSection />
                    </Match>
                    <Match when={section() === "advanced"}>
                        <AdvancedSection />
                    </Match>
                </Switch>
                <footer class="settings-footer">
                    <button class="settings-footer-btn" onClick={() => void openRaw()}>
                        <i class="fa-solid fa-file-code" /> Open raw settings.json
                    </button>
                </footer>
            </div>
            <nav class="settings-tab-bar" aria-label="Settings section">
                <For each={RAIL}>
                    {(item) => (
                        <button
                            type="button"
                            aria-label={item.label}
                            classList={{ "is-active": section() === item.id }}
                            aria-pressed={section() === item.id}
                            onClick={() => setSection(item.id)}
                        >
                            <i class={`fa-solid fa-${item.icon}`} aria-hidden="true" />
                        </button>
                    )}
                </For>
            </nav>
        </div>
        </div>
    );
}
