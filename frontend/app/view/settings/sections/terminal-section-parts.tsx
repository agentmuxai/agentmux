// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The groups of Settings → Terminal (terminal-section.tsx lays them out): the shell and the text, what the keys do,
// SSH, the display, predictive echo and the agent watchdog. Each group is its rows, returned as a fragment.

import { Select, TextInput } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";

import { fullConfigAtom, settingsAtom } from "@/app/store/global";
import { NumberControl, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";
import { DefaultShellSetting } from "./default-shell";
import { TERMINAL_SETTINGS } from "./terminal-settings-index";

const s = () => settingsAtom() ?? ({} as any);

/** The default shell, the font, the color theme and the scrollback. */
export function ShellAndTextGroup(): JSX.Element {
    const termThemes = () => Object.entries((fullConfigAtom()?.termthemes as Record<string, any>) ?? {})
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));
    return (
        <>
            <SettingRow
                id={TERMINAL_SETTINGS.defaultShell.id}
                label={TERMINAL_SETTINGS.defaultShell.label}
                description={TERMINAL_SETTINGS.defaultShell.description}
                stacked
                control={<DefaultShellSetting />}
            />
            <SettingRow
                id={TERMINAL_SETTINGS.fontSize.id}
                label={TERMINAL_SETTINGS.fontSize.label}
                description={TERMINAL_SETTINGS.fontSize.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={8} max={32} step={1} parse="int"
                        value={(s()["term:fontsize"] as number) ?? 15}
                        onChange={(v) => set("term:fontsize", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.fontFamily.id}
                label={TERMINAL_SETTINGS.fontFamily.label}
                description={TERMINAL_SETTINGS.fontFamily.description}
                control={
                    <TextInput
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
                        <Select
                            class="setting-select"
                            value={(s()["term:theme"] as string) ?? ""}
                            onChange={(v) => set("term:theme", v || null)}
                        >
                            <option value="">Default</option>
                            <For each={termThemes()}>
                                {([key, theme]) => <option value={key}>{theme["display:name"] ?? key}</option>}
                            </For>
                        </Select>
                    }
                />
            </Show>
            <SettingRow
                id={TERMINAL_SETTINGS.scrollback.id}
                label={TERMINAL_SETTINGS.scrollback.label}
                description={TERMINAL_SETTINGS.scrollback.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={1000} max={100000} step={1000} parse="int"
                        value={(s()["term:scrollback"] as number) ?? 2000}
                        onChange={(v) => set("term:scrollback", v)}
                    />
                }
            />
        </>
    );
}

/** What selecting and Shift+Enter do. */
export function KeysGroup(): JSX.Element {
    return (
        <>
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
        </>
    );
}

/** Durable SSH sessions and the helper install. */
export function SshGroup(): JSX.Element {
    return (
        <>
            <SettingRow
                id={TERMINAL_SETTINGS.durableSsh.id}
                label={TERMINAL_SETTINGS.durableSsh.label}
                description={TERMINAL_SETTINGS.durableSsh.description}
                control={
                    <Select
                        class="setting-select"
                        value={
                            s()["term:durable"] === true ? "on" : s()["term:durable"] === false ? "off" : "auto"
                        }
                        onChange={(v) => {
                            // "auto" clears the setting: srv's default applies.
                            set("term:durable", v === "on" ? true : v === "off" ? false : null);
                        }}
                    >
                        <option value="auto">On, else a plain terminal</option>
                        <option value="on">Always (needs the helper)</option>
                        <option value="off">Never</option>
                    </Select>
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.helperInstall.id}
                label={TERMINAL_SETTINGS.helperInstall.label}
                description={TERMINAL_SETTINGS.helperInstall.description}
                control={
                    <Select
                        class="setting-select"
                        value={(s()["conn:helper"] as string | undefined) || "ask"}
                        onChange={(v) => {
                            // "ask" clears the setting: asking is the default.
                            set("conn:helper", v === "ask" ? null : v);
                        }}
                    >
                        <option value="ask">Ask first</option>
                        <option value="always">Always</option>
                        <option value="never">Never</option>
                    </Select>
                }
            />
        </>
    );
}

/** Bracketed paste, transparency, scroll sensitivity and the CPU/mem badge. */
export function DisplayGroup(): JSX.Element {
    return (
        <>
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
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={0.1} max={10} step={0.1}
                        value={(s()["term:scrollsensitivity"] as number) ?? 1}
                        onChange={(v) => set("term:scrollsensitivity", v)}
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
        </>
    );
}

/** Predictive echo and its latency threshold. */
export function EchoGroup(): JSX.Element {
    return (
        <>
            <SettingRow
                id={TERMINAL_SETTINGS.predictiveEcho.id}
                label={TERMINAL_SETTINGS.predictiveEcho.label}
                description={TERMINAL_SETTINGS.predictiveEcho.description}
                control={
                    <ToggleControl
                        checked={s()["term:predictiveecho"] !== false}
                        onChange={(v) => set("term:predictiveecho", v)}
                    />
                }
            />
            <Show when={s()["term:predictiveecho"] !== false}>
                <SettingRow
                    id={TERMINAL_SETTINGS.predictiveEchoThreshold.id}
                    indent
                    label={TERMINAL_SETTINGS.predictiveEchoThreshold.label}
                    description={TERMINAL_SETTINGS.predictiveEchoThreshold.description}
                    control={
                        <NumberControl
                            class="setting-number setting-number--wide"
                            min={0} step={1}
                            value={(s()["term:predictiveecho:thresholdms"] as number) ?? 0}
                            onChange={(v) => set("term:predictiveecho:thresholdms", v)}
                        />
                    }
                />
            </Show>
        </>
    );
}

/** How long the watchdog lets an agent pane run or sit idle. */
export function WatchdogGroup(): JSX.Element {
    return (
        <>
            <SettingRow
                id={TERMINAL_SETTINGS.agentMaxRuntime.id}
                label={TERMINAL_SETTINGS.agentMaxRuntime.label}
                description={TERMINAL_SETTINGS.agentMaxRuntime.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={0} step={0.5}
                        value={(s()["term:agentmaxruntimehours"] as number) ?? 0}
                        onChange={(v) => set("term:agentmaxruntimehours", v)}
                    />
                }
            />
            <SettingRow
                id={TERMINAL_SETTINGS.agentIdleTimeout.id}
                label={TERMINAL_SETTINGS.agentIdleTimeout.label}
                description={TERMINAL_SETTINGS.agentIdleTimeout.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={0} step={1}
                        value={(s()["term:agentidletimeoutmins"] as number) ?? 0}
                        onChange={(v) => set("term:agentidletimeoutmins", v)}
                    />
                }
            />
        </>
    );
}
