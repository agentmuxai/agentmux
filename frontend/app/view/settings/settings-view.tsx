// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, For, on, Show, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";

import { Button, TabbedPane, type TabItem } from "@/app/element/ui";

import { t } from "@/app/i18n/i18n";
import { fullConfigAtom } from "@/app/store/global";
import { openConfigFileOrTell, type ConfigFile } from "@/app/util/open-config-file";
import { SETTINGS_SECTION_LABELS, SETTINGS_SECTIONS, type SettingsIndexEntry, type SettingsSection, type SettingsViewModel } from "./settings-model";
import { SettingsSearchBar } from "./settings-search-bar";
import { AppearanceSection } from "./sections/appearance-section";
import { AnimationsSection } from "./sections/animations-section";
import { WindowPanesSection } from "./sections/window-panes-section";
import { BrowserSection } from "./sections/browser-section";
import { TerminalSection } from "./sections/terminal-section";
import { KeyboardSection } from "./sections/keyboard-section";
import { SoundsSection } from "./sections/sounds-section";
import { NotificationsSection } from "./sections/notifications-section";
import { RecordingSection } from "./sections/recording-section";
import { DevicesSection } from "./sections/devices-section";
import { WidgetsSection } from "./sections/widgets-section";
import { AgentsSection } from "./sections/agents-section";
import { KanbanSection } from "./sections/kanban-section";
import { SupervisorSection } from "./sections/supervisor-section";
import { AdvancedSection } from "./sections/advanced-section";
import "./settings.scss";
import { DURATION_LINGER_MS } from "@/app/motion/tokens";
import { focusWhenRendered } from "@/util/focusutil";

// ── Config error banner ───────────────────────────────────────────────────────

/** An error that carries a line and does not already say so in its text (a bad value does not; a parse error does). */
function lineToShow(e: { err: string; line?: number }): number | undefined {
    return e.line != null && !/\bline \d/i.test(e.err) ? e.line : undefined;
}

export function ConfigErrorsBanner(props: { onOpen: (file: ConfigFile, line?: number) => void }): JSX.Element {
    const errors = () => fullConfigAtom()?.configerrors ?? [];
    // The file the first error is about: a mistake in widgets.json opens widgets.json, anything else settings.json, and
    // the Editor opens it on that error's line when srv knows it.
    const openRaw = () => props.onOpen(/widgets\.json$/i.test(errors()[0]?.file ?? "") ? "widgets.json" : "settings.json", errors()[0]?.line);
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
                            <Show when={lineToShow(e)}>
                                {(line) => <>{" "}<span class="mono">{t("settings.errors.line", { line: line() })}</span></>}
                            </Show>
                        </div>
                    )}
                </For>
                <Button tone="danger" class="settings-config-error-fix" onClick={() => void openRaw()}>
                    {t("settings.errors.fix")}
                </Button>
            </div>
        </Show>
    );
}

// ── Rail ──────────────────────────────────────────────────────────────────────

const RAIL: TabItem<SettingsSection>[] = [...SETTINGS_SECTIONS.items];

// The body each rail entry shows. A section mounts when its tab is chosen and unmounts when another is.
const SECTION_VIEWS: Record<SettingsSection, (p: { model: SettingsViewModel }) => JSX.Element> = {
    appearance: AppearanceSection,
    animations: AnimationsSection,
    window: WindowPanesSection,
    browser: BrowserSection,
    terminal: TerminalSection,
    keyboard: KeyboardSection,
    sounds: SoundsSection,
    notifications: NotificationsSection,
    recording: RecordingSection,
    devices: DevicesSection,
    widgets: WidgetsSection,
    agents: AgentsSection,
    supervisor: SupervisorSection,
    kanban: KanbanSection,
    advanced: AdvancedSection,
};

// ── Main view ─────────────────────────────────────────────────────────────────

// Brief on-screen confirmation that a search result actually landed on the
// right row — the CSS class does the fade, this just applies/removes it.
// docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §3.5.
const SEARCH_HIGHLIGHT_MS = DURATION_LINGER_MS; // --duration-linger, which the fade runs for
const SEARCH_HIGHLIGHT_CLASS = "setting-row--search-highlight";
/** How long a link's setting row is waited for before the section is left at its top (§8 of the deep-links spec). */
const REVEAL_WAIT_MS = 5000;

/** Scroll to a setting row and mark it, as a search result does. */
function markRow(el: HTMLElement): void {
    el.scrollIntoView({ block: "center" });
    el.classList.add(SEARCH_HIGHLIGHT_CLASS);
    setTimeout(() => el.classList.remove(SEARCH_HIGHLIGHT_CLASS), SEARCH_HIGHLIGHT_MS);
}

/**
 * A link to a setting (agentmux://settings/<section>#<id>): the same reveal as a search result, once the section has
 * drawn the row; a row that never appears within REVEAL_WAIT_MS leaves the section at its top. Called in a component.
 */
function revealLinkedSetting(model: SettingsViewModel): void {
    createEffect(
        on(
            // Keyed by the ask, so another change to the pane's meta is not a new ask.
            () => {
                const r = model.revealRequest();
                return r ? `${r.ask}:${r.id}` : null;
            },
            (key) => {
                const req = key ? model.revealRequest() : null;
                if (!req) return;
                const until = performance.now() + REVEAL_WAIT_MS;
                const tryReveal = () => {
                    const el = document.getElementById(`setting-${req.id}`);
                    if (el) {
                        markRow(el);
                        model.clearReveal();
                    } else if (performance.now() < until) {
                        requestAnimationFrame(tryReveal);
                    } else {
                        model.clearReveal();
                    }
                };
                queueMicrotask(tryReveal);
            }
        )
    );
}

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
        focusWhenRendered(() => container.querySelector<HTMLInputElement>("[data-testid='settings-search-input']"), { select: true });
    };

    const openFile = (file: ConfigFile, line?: number) => openConfigFileOrTell(file, { line });
    const openRaw = () => openFile("settings.json");

    function handleSelectResult(entry: SettingsIndexEntry) {
        if (entry.opens) {
            entry.opens();
            return;
        }
        props.model.setSection(entry.section);
        // The target section's row only exists in the DOM once its section
        // arm has (re-)rendered for the new section — queueMicrotask, not a
        // same-tick lookup, mirrors the same pattern the command palette
        // uses for its own post-mount focus() (command-palette.tsx).
        queueMicrotask(() => {
            const el = document.getElementById(`setting-${entry.id}`);
            if (el) markRow(el);
        });
    }

    revealLinkedSetting(props.model);

    return (
        <div
            ref={container}
            class="settings-view-container"
            style={{ zoom: props.model.zoom() }}
            onPointerDown={() => {
                lastPressAt = performance.now();
            }}
        >
            {/* One tablist along the top: icons only when narrow, labels when
                there's room (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4). */}
            <TabbedPane items={RAIL} value={section()} onChange={setSection} ariaLabel={t("settings.rail.label")}>
                <div class="settings-body">
                    <SettingsSearchBar
                        query={props.model.query}
                        setQuery={props.model.setQuery}
                        onSelectResult={handleSelectResult}
                    />
                    <ConfigErrorsBanner onOpen={(f, line) => void openFile(f, line)} />
                    {/* The scroller. A section that flows into columns sits in it with an auto height, so the
                        browser balances the columns down instead of paging them to the right; one wrapper per
                        section, so a new section starts at the top. */}
                    <Show when={section()} keyed>
                        {(current) => (
                            <div class="settings-scroll">
                                <Dynamic component={SECTION_VIEWS[current]} model={props.model} />
                            </div>
                        )}
                    </Show>
                    <footer class="settings-footer">
                        <Button icon="file-code" onClick={() => void openRaw()}>
                            {t("settings.footer.openRaw", { file: "settings.json" })}
                        </Button>
                        <Button icon="file-code" onClick={() => void openFile("widgets.json")}>
                            {t("settings.footer.openRaw", { file: "widgets.json" })}
                        </Button>
                    </footer>
                </div>
            </TabbedPane>
        </div>
    );
}
