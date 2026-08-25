// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Match, Show, Switch, type JSX } from "solid-js";

import { fullConfigAtom } from "@/app/store/global";
import { invokeCommand } from "@/app/platform/ipc";
import { SETTINGS_SECTION_LABELS, type SettingsSection, type SettingsViewModel } from "./settings-model";
import { AppearanceSection } from "./sections/appearance-section";
import { WindowPanesSection } from "./sections/window-panes-section";
import { TerminalSection } from "./sections/terminal-section";
import { SoundsSection } from "./sections/sounds-section";
import { RecordingSection } from "./sections/recording-section";
import { AdvancedSection } from "./sections/advanced-section";
import "./settings.scss";

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

// ── Rail ──────────────────────────────────────────────────────────────────────

const RAIL: { id: SettingsSection; label: string; icon: string }[] = [
    { id: "appearance", label: SETTINGS_SECTION_LABELS.appearance, icon: "palette" },
    { id: "window",     label: SETTINGS_SECTION_LABELS.window,     icon: "table-cells" },
    { id: "terminal",   label: SETTINGS_SECTION_LABELS.terminal,   icon: "square-terminal" },
    { id: "sounds",     label: SETTINGS_SECTION_LABELS.sounds,     icon: "volume-high" },
    { id: "recording",  label: SETTINGS_SECTION_LABELS.recording,  icon: "microphone" },
    { id: "advanced",   label: SETTINGS_SECTION_LABELS.advanced,   icon: "sliders" },
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
            {/* Narrow-width fallback for .settings-rail below — rendered FIRST
                (not last) so it sits at the top of the pane, not the bottom, at
                the same breakpoint the rail hides. See
                docs/specs/SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md. */}
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
                    <Match when={section() === "recording"}>
                        <RecordingSection />
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
        </div>
        </div>
    );
}
