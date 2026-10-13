// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Widgets → Widget bar: every entry the widget bar knows about, pinned on the bar,
// kept in More, or hidden from both (and from the pane "+" picker). Reads the same config and
// the same model as the bar (getWidgetBarModel), with no copy of its own, so a change on the bar,
// in widgets.json or in settings.json shows here at once, and a change here shows on the bar.
// docs/specs/SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md §4.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, IconButton, SegmentedControl, type SegmentedOption } from "@/app/element/ui";
import {
    getWidgetBarModel,
    movePinnedWidget,
    resetWidgetBar,
    setWidgetBarState,
    widgetIconStyle,
    type WidgetBarEntry,
    type WidgetBarState,
} from "@/app/window/action-widgets-config";
import { atoms } from "@/store/global";
import { makeIconClass } from "@/util/util";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

import "./widget-bar-settings.scss";

export const WIDGET_BAR_SETTINGS = {
    bar: {
        id: "widgets.bar",
        label: "Widget bar",
        description:
            "Choose what the widget bar shows. Pinned widgets are icons on the bar, More keeps them in the More menu, and Hidden removes them from both and from a pane's + menu.",
        section: "widgets",
        keywords: ["widget bar", "toolbar", "pin", "unpin", "hide", "show", "more menu", "+ menu", "reorder", "widget:pinned", "widget:hidden"],
    },
    iconOnly: {
        id: "widgets.bar.icononly",
        label: "Show icons only",
        description: "Icons without text labels on the widget bar",
        section: "widgets",
        keywords: ["compact widget bar", "hide labels", "icon-only", "widget:icononly"],
    },
    reset: {
        id: "widgets.bar.reset",
        label: "Reset widget bar",
        description: "Back to the default pinned widgets, with nothing hidden",
        section: "widgets",
        keywords: ["default widgets", "restore", "unhide all"],
    },
} satisfies Record<string, SettingsIndexEntry>;

function topLevelOptions(entry: WidgetBarEntry): SegmentedOption<WidgetBarState>[] {
    return [
        { value: "pinned", label: "Pinned", disabled: !entry.canPin, title: entry.canPin ? undefined : "Installed widgets can't be pinned yet" },
        { value: "more", label: "More" },
        { value: "hidden", label: "Hidden", disabled: !entry.canHide },
    ];
}

function childOptions(entry: WidgetBarEntry): SegmentedOption<WidgetBarState>[] {
    return [
        { value: "group", label: "In group" },
        { value: "pinned", label: "Pinned" },
        { value: "hidden", label: "Hidden", disabled: !entry.canHide },
    ];
}

/** "from package acme.board" for an installed widget's entry (`ext@acme.board/main`). */
function sourceNote(key: string): string | null {
    const m = /^ext@([^/]+)\//.exec(key);
    return m ? `from package ${m[1]}` : null;
}

export function WidgetBarSettings(): JSX.Element {
    const settings = (): Record<string, any> => atoms.fullConfigAtom()?.settings ?? {};
    const wmap = (): Record<string, WidgetConfigType> => atoms.fullConfigAtom()?.widgets ?? {};
    const model = () => getWidgetBarModel(settings(), wmap());
    const pinnedCount = () => model().filter((e) => e.state === "pinned").length;
    const [error, setError] = createSignal<string | null>(null);

    // Every write goes through SetConfigCommand; the config event that follows re-renders this
    // list and the bar together, so there is no local state to keep in step.
    const write = (f: () => Promise<void>) => {
        f().then(
            () => setError(null),
            (e: unknown) => setError(e instanceof Error ? e.message : String(e))
        );
    };
    const setState = (entry: WidgetBarEntry, state: WidgetBarState) =>
        write(() => setWidgetBarState(entry.key, state, settings(), wmap()));

    const Row = (p: { entry: WidgetBarEntry; index?: number; child?: boolean }): JSX.Element => (
        <div
            class="widget-bar-row"
            classList={{ "widget-bar-row--child": !!p.child, "widget-bar-row--hidden": p.entry.state === "hidden" }}
            data-widget-key={p.entry.key}
        >
            <span class="widget-bar-row-icon widget-icon" style={widgetIconStyle(p.entry.widget)}>
                <i class={makeIconClass(p.entry.widget.icon, true, { defaultIcon: "window-maximize" })} />
            </span>
            <span class="widget-bar-row-label">
                {p.entry.widget.label ?? p.entry.shortName}
                <Show when={sourceNote(p.entry.key)}>{(note) => <span class="widget-bar-row-note">{note()}</span>}</Show>
                <Show when={!p.entry.canHide}>
                    <span class="widget-bar-row-note">always available</span>
                </Show>
            </span>
            <span class="widget-bar-row-move">
                <Show when={!p.child && p.entry.state === "pinned"}>
                    <IconButton
                        icon="arrow-up"
                        label="Move left on the bar"
                        disabled={p.index === 0}
                        onClick={() => write(() => movePinnedWidget(p.entry.shortName, -1, settings(), wmap()))}
                    />
                    <IconButton
                        icon="arrow-down"
                        label="Move right on the bar"
                        disabled={p.index === pinnedCount() - 1}
                        onClick={() => write(() => movePinnedWidget(p.entry.shortName, 1, settings(), wmap()))}
                    />
                </Show>
            </span>
            <SegmentedControl
                density="compact"
                ariaLabel={`${p.entry.widget.label ?? p.entry.shortName} on the widget bar`}
                options={p.child ? childOptions(p.entry) : topLevelOptions(p.entry)}
                value={p.entry.state}
                onChange={(state) => setState(p.entry, state)}
            />
        </div>
    );

    return (
        <>
            <SectionHeader label={WIDGET_BAR_SETTINGS.bar.label} />
            <div id={`setting-${WIDGET_BAR_SETTINGS.bar.id}`} class="setting-row widget-bar-settings">
                <div class="setting-devices-description">{WIDGET_BAR_SETTINGS.bar.description}</div>
                <div class="widget-bar-list" role="list" aria-label="Widget bar entries">
                    <For each={model()}>
                        {(entry, i) => (
                            <div role="listitem">
                                <Row entry={entry} index={i()} />
                                <For each={entry.children}>{(child) => <Row entry={child} child />}</For>
                            </div>
                        )}
                    </For>
                </div>
                <Show when={error()}>
                    {(msg) => (
                        <div class="settings-config-error" role="alert">
                            {msg()}
                        </div>
                    )}
                </Show>
            </div>
            <SettingRow
                id={WIDGET_BAR_SETTINGS.iconOnly.id}
                label={WIDGET_BAR_SETTINGS.iconOnly.label}
                description={WIDGET_BAR_SETTINGS.iconOnly.description}
                control={
                    <ToggleControl
                        checked={!!(settings()["widget:icononly"] as boolean)}
                        onChange={(v) => set("widget:icononly", v)}
                    />
                }
            />
            <SettingRow
                id={WIDGET_BAR_SETTINGS.reset.id}
                label={WIDGET_BAR_SETTINGS.reset.label}
                description={WIDGET_BAR_SETTINGS.reset.description}
                control={
                    <Button icon="rotate-left" onClick={() => write(resetWidgetBar)}>
                        Reset
                    </Button>
                }
            />
        </>
    );
}
