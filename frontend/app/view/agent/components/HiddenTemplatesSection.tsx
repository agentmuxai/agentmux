// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HiddenTemplatesSection — Phase 2 (Q2 Decision Y) of the two-tier
 * picker (`SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`).
 *
 * A collapsible "Hidden templates" section that lives directly under
 * the picker's templates tier. Lists templates the user has previously
 * hidden via right-click → "Hide template", and offers an Unhide
 * button per row to bring them back into the main picker.
 *
 * Why this lives in AgentPicker rather than a separate settings
 * panel: per the project CLAUDE.md note, the hamburger Settings menu
 * just opens `settings.json` in the user's editor — AgentMux doesn't
 * have an in-app settings panel that's a natural home for "manage
 * hidden agent templates". Co-locating hide + unhide on the picker
 * surface keeps the affordance discoverable: where you hid it is
 * where you go to bring it back.
 *
 * Data source:
 *  - `AgentDefListHiddenTemplatesCommand` — backend-filtered list of
 *    rows with `is_seeded = 1 AND user_hidden = 1`.
 *  - Subscribes to `agents:changed` so hide/unhide elsewhere keeps
 *    this section's count + list in sync.
 *
 * UX:
 *  - Empty case: section is collapsed AND the header is hidden
 *    entirely — zero footprint when nothing's hidden, so first-run
 *    users don't see an empty curiosity.
 *  - Non-empty: header shows `Hidden templates (N)`, click to expand.
 *    Each row: icon + name + Unhide button.
 */

import {
    createEffect,
    createMemo,
    createSignal,
    For,
    onCleanup,
    Show,
    type JSX,
} from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { ProviderLogo } from "@/element/ProviderLogo";

export const HiddenTemplatesSection = (): JSX.Element => {
    const [hidden, setHidden] = createSignal<AgentDefinition[]>([]);
    const [expanded, setExpanded] = createSignal(false);
    const [loading, setLoading] = createSignal(false);

    let cancelled = false;
    const load = async () => {
        try {
            setLoading(true);
            const rows = await RpcApi.AgentDefListHiddenTemplatesCommand(TabRpcClient);
            if (!cancelled) setHidden(rows ?? []);
        } catch {
            // Silent — empty result is the safe fallback. The picker
            // proper still works without this section.
            if (!cancelled) setHidden([]);
        } finally {
            if (!cancelled) setLoading(false);
        }
    };

    // Initial load + re-fetch on every `agents:changed`. The backend
    // fires that event from both `agentdefhide` and `agentdefunhide`,
    // so any state change reflects here without a manual refresh.
    void load();
    const unsub = waveEventSubscribe({
        eventType: "agents:changed",
        handler: () => void load(),
    });
    onCleanup(() => {
        cancelled = true;
        unsub();
    });

    // Auto-collapse when the list becomes empty, so the next time the
    // section becomes non-empty the user sees the header in its
    // collapsed state (no surprise "Hidden templates" panel
    // pre-opened).
    createEffect(() => {
        if (hidden().length === 0 && expanded()) {
            setExpanded(false);
        }
    });

    const handleUnhide = async (agent: AgentDefinition) => {
        try {
            await RpcApi.AgentDefUnhideCommand(TabRpcClient, {
                definition_id: agent.id,
            });
            // The waveEvent refresh will repopulate, but optimistic
            // local update keeps the click snappy.
            setHidden((rows) => rows.filter((r) => r.id !== agent.id));
        } catch (err) {
            // eslint-disable-next-line no-console
            console.warn(`agentdefunhide failed for ${agent.id}:`, err);
        }
    };

    const count = createMemo(() => hidden().length);

    return (
        <Show when={count() > 0}>
            <div
                class="agent-picker-hidden-templates"
                data-testid="agent-hidden-templates-section"
            >
                <button
                    type="button"
                    class="agent-picker-hidden-templates-header"
                    aria-expanded={expanded()}
                    onClick={() => setExpanded((v) => !v)}
                    data-testid="agent-hidden-templates-toggle"
                >
                    <span class="agent-picker-hidden-templates-caret">
                        {expanded() ? "▾" : "▸"}
                    </span>
                    <span>Hidden templates ({count()})</span>
                </button>
                <Show when={expanded()}>
                    <div
                        class="agent-picker-hidden-templates-body"
                        data-testid="agent-hidden-templates-list"
                    >
                        <Show
                            when={!loading()}
                            fallback={
                                <div class="agent-picker-hidden-templates-loading">
                                    Loading…
                                </div>
                            }
                        >
                            <Show
                                when={count() > 0}
                                fallback={
                                    <div class="agent-picker-hidden-templates-empty">
                                        No hidden templates.
                                    </div>
                                }
                            >
                                <For each={hidden()}>
                                    {(agent) => (
                                        <div
                                            class="agent-picker-hidden-template-row"
                                            data-testid={`agent-hidden-template-${agent.id}`}
                                        >
                                            <ProviderLogo
                                                provider={agent.provider}
                                                size={20}
                                                class="agent-picker-hidden-template-icon"
                                            />
                                            <span class="agent-picker-hidden-template-name">
                                                {agent.name}
                                            </span>
                                            <button
                                                type="button"
                                                class="agent-picker-hidden-template-unhide-btn"
                                                onClick={() => void handleUnhide(agent)}
                                                data-testid={`agent-hidden-template-unhide-${agent.id}`}
                                                aria-label={`Unhide template ${agent.name}`}
                                            >
                                                Unhide
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </Show>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

HiddenTemplatesSection.displayName = "HiddenTemplatesSection";
