// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Widgets: every installed widget package, its state and
// permissions, and installing, approving, enabling and removing them
// (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.4).
//
// Approving is the user's decision in this UI, carried to srv by the host
// (`approvals.decideWidget`): an agent, which has srv's auth key, can install a
// package but can't approve one.

import { type JSX, Show } from "solid-js";

import { setWidgetPackages } from "@/app/store/widget-packages-store";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader } from "../settings-controls";
import { WidgetCatalog } from "./widget-catalog";
import { WidgetBarSettings } from "./widget-bar-settings";
import { Publishers, ReplacePrompt, WidgetList, WidgetsToolbar } from "./widgets-section-parts";
import { createWidgetsSection } from "./widgets-section-state";

export const WIDGETS_SETTINGS = {
    installed: {
        id: "widgets.installed",
        label: "Widgets",
        description:
            "Widgets add your own kinds of pane. A widget runs only after you approve it here, and asks again whenever its files change.",
        section: "widgets",
        keywords: ["widget", "plugin", "extension", "install", "approve", "ext:", "sandbox", "custom pane", "widget.json"],
    },
} satisfies Record<string, SettingsIndexEntry>;

export function WidgetsSection(): JSX.Element {
    const s = createWidgetsSection();
    const { error, setAsking } = s;
    return (
        <div class="settings-section-body widgets-section">
            <WidgetBarSettings />
            <SectionHeader label={WIDGETS_SETTINGS.installed.label} />
            <div id={`setting-${WIDGETS_SETTINGS.installed.id}`} class="setting-row">
                <div class="setting-devices-description">{WIDGETS_SETTINGS.installed.description}</div>
                <WidgetsToolbar s={s} />
                <div class="setting-devices-description">
                    Pick a package's widget.json, or a .zip. To write your own, start from a sample in docs/examples/widgets.
                </div>
                <WidgetCatalog
                    onInstalled={(id, packages) => {
                        setWidgetPackages(packages);
                        setAsking(id);
                    }}
                />
                <ReplacePrompt s={s} />
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <WidgetList s={s} />
                <Publishers s={s} />
            </div>
        </div>
    );
}
