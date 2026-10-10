// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Match, Show, Switch, type JSX } from "solid-js";

import { Button, TabbedPane, type TabItem } from "@/app/element/ui";

import { fullConfigAtom } from "@/app/store/global";
import { getApi } from "@/app/store/app-api";
import { SETTINGS_SECTION_LABELS, type SettingsIndexEntry, type SettingsSection, type SettingsViewModel } from "./settings-model";
import { SettingsSearchBar } from "./settings-search-bar";
import { AppearanceSection } from "./sections/appearance-section";
import { WindowPanesSection } from "./sections/window-panes-section";
import { BrowserSection } from "./sections/browser-section";
import { TerminalSection } from "./sections/terminal-section";
import { SoundsSection } from "./sections/sounds-section";
import { NotificationsSection } from "./sections/notifications-section";
import { RecordingSection } from "./sections/recording-section";
import { DevicesSection } from "./sections/devices-section";
import { WidgetsSection } from "./sections/widgets-section";
import { AdvancedSection } from "./sections/advanced-section";
import "./settings.scss";
import { focusWhenRendered } from "@/util/focusutil";

// ── Config error banner ───────────────────────────────────────────────────────

function ConfigErrorsBanner(): JSX.Element {
    const errors = () => fullConfigAtom()?.configerrors ?? [];
    const openRaw = () => getApi().openSettingsFileInEditor();
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
                <Button tone="danger" class="settings-config-error-fix" onClick={() => void openRaw()}>
                    Fix in editor
                </Button>
            </div>
        </Show>
    );
}

// ── Rail ──────────────────────────────────────────────────────────────────────

const RAIL: TabItem<SettingsSection>[] = [
    { id: "appearance", label: SETTINGS_SECTION_LABELS.appearance, icon: "palette" },
    { id: "window",     label: SETTINGS_SECTION_LABELS.window,     icon: "table-cells" },
    { id: "browser",    label: SETTINGS_SECTION_LABELS.browser,    icon: "globe" },
    { id: "terminal",   label: SETTINGS_SECTION_LABELS.terminal,   icon: "square-terminal" },
    { id: "sounds",     label: SETTINGS_SECTION_LABELS.sounds,     icon: "volume-high" },
    { id: "notifications", label: SETTINGS_SECTION_LABELS.notifications, icon: "bell" },
    { id: "recording",  label: SETTINGS_SECTION_LABELS.recording,  icon: "microphone" },
    { id: "devices",    label: SETTINGS_SECTION_LABELS.devices,    icon: "mobile-screen" },
    { id: "widgets",    label: SETTINGS_SECTION_LABELS.widgets,    icon: "puzzle-piece" },
    { id: "advanced",   label: SETTINGS_SECTION_LABELS.advanced,   icon: "sliders" },
];

// ── Main view ─────────────────────────────────────────────────────────────────

// Brief on-screen confirmation that a search result actually landed on the
// right row — the CSS class does the fade, this just applies/removes it.
// docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §3.5.
const SEARCH_HIGHLIGHT_MS = 1500;
const SEARCH_HIGHLIGHT_CLASS = "setting-row--search-highlight";

export function SettingsView(props: { model: SettingsViewModel }): JSX.Element {
    const section = () => props.model.activeSection();
    let container!: HTMLDivElement;
    let lastPressAt = Number.NEGATIVE_INFINITY;
    // A section picked with the mouse hands the caret back to the search, its
    // text selected, so the next keystrokes search. Arrow keys in the tab list
    // keep it there, so keyboard navigation of the tabs still works.
    const setSection = (s: SettingsSection) => {
        props.model.setSection(s);
        if (performance.now() - lastPressAt > 1000) return;
        focusWhenRendered(() => container.querySelector<HTMLInputElement>(".settings-search-input"), { select: true });
    };

    const openRaw = () => getApi().openSettingsFileInEditor();

    function handleSelectResult(entry: SettingsIndexEntry) {
        props.model.setSection(entry.section);
        // The target section's row only exists in the DOM once its <Match>
        // arm has (re-)rendered for the new section — queueMicrotask, not a
        // same-tick lookup, mirrors the same pattern the command palette
        // uses for its own post-mount focus() (command-palette.tsx).
        queueMicrotask(() => {
            const el = document.getElementById(`setting-${entry.id}`);
            if (!el) return;
            el.scrollIntoView({ block: "center" });
            el.classList.add(SEARCH_HIGHLIGHT_CLASS);
            setTimeout(() => el.classList.remove(SEARCH_HIGHLIGHT_CLASS), SEARCH_HIGHLIGHT_MS);
        });
    }

    return (
        <div
            ref={container}
            class="settings-view-container"
            onPointerDown={() => {
                lastPressAt = performance.now();
            }}
        >
            {/* One tablist along the top: icons only when narrow, labels when
                there's room (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4). */}
            <TabbedPane items={RAIL} value={section()} onChange={setSection} ariaLabel="Settings section">
                <div class="settings-body">
                    <SettingsSearchBar
                        query={props.model.query}
                        setQuery={props.model.setQuery}
                        onSelectResult={handleSelectResult}
                    />
                    <ConfigErrorsBanner />
                    <Switch>
                        <Match when={section() === "appearance"}>
                            <AppearanceSection />
                        </Match>
                        <Match when={section() === "window"}>
                            <WindowPanesSection />
                        </Match>
                        <Match when={section() === "browser"}>
                            <BrowserSection />
                        </Match>
                        <Match when={section() === "terminal"}>
                            <TerminalSection />
                        </Match>
                        <Match when={section() === "sounds"}>
                            <SoundsSection />
                        </Match>
                        <Match when={section() === "notifications"}>
                            <NotificationsSection />
                        </Match>
                        <Match when={section() === "recording"}>
                            <RecordingSection />
                        </Match>
                        <Match when={section() === "devices"}>
                            <DevicesSection />
                        </Match>
                        <Match when={section() === "widgets"}>
                            <WidgetsSection />
                        </Match>
                        <Match when={section() === "advanced"}>
                            <AdvancedSection />
                        </Match>
                    </Switch>
                    <footer class="settings-footer">
                        <Button icon="file-code" onClick={() => void openRaw()}>
                            Open raw settings.json
                        </Button>
                    </footer>
                </div>
            </TabbedPane>
        </div>
    );
}
