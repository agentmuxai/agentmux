// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ForkBar — the bottom-of-pane "fork bar": one row per conversation fork in
 * this pane, the active one promoted. Click (or ↓/↑ — wired later) loads a
 * fork; `+` forks the current conversation; `⌫` closes a fork.
 *
 * Presentational: it renders a fork set (from `computeForkSet`) via the shared
 * `<PaneRow>` chrome and reports user intent through callbacks. It owns no data
 * and performs no switching — the caller wires `onSwitch` to the block-stack
 * swap and `onFork` to the `/btw` flow. Registers into the `forks` region of
 * `<PaneRegions>`. Spec: SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §7.
 */

import { For, Show, type JSX } from "solid-js";
import { PaneRow, type PaneRowAccent } from "../components/PaneRow";
import type { ForkSetEntry } from "./fork-set";
import "./ForkBar.scss";

export interface ForkBarProps {
    /** The fork set for this pane (from `computeForkSet`), root first. */
    forks: ForkSetEntry[];
    /** Load a fork — switch the pane to that conversation. */
    onSwitch: (definitionId: string) => void;
    /** Fork the active conversation into a new sibling (the `+` affordance). */
    onFork?: () => void;
    /** Close a fork (never offered for the root — that's closing the pane). */
    onClose?: (definitionId: string) => void;
}

/** Status accent for a fork row: active is promoted; an open-but-inactive fork
 *  reads `running` (live in the background); a not-open fork reads `idle`. */
function accentFor(f: ForkSetEntry): PaneRowAccent {
    if (f.isActive) return "active";
    return f.blockId ? "running" : "idle";
}

export const ForkBar = (props: ForkBarProps): JSX.Element => {
    // A single-conversation pane (root only) shows no bar — zero cost for the
    // common case; the bar appears once a second fork exists.
    return (
        <Show when={props.forks.length > 1}>
            <div class="fork-bar" role="tablist" aria-label="Conversation forks">
                <For each={props.forks}>
                    {(f) => (
                        <div role="tab" aria-selected={f.isActive}>
                            <PaneRow
                                sigil={f.isActive ? "▣" : "⑂"}
                                title={f.title}
                                accent={accentFor(f)}
                                onActivate={() => props.onSwitch(f.definitionId)}
                                actions={
                                    !f.isRoot && props.onClose
                                        ? [{
                                            glyph: "⌫",
                                            title: `Close ${f.title}`,
                                            danger: true,
                                            onClick: () => props.onClose!(f.definitionId),
                                        }]
                                        : []
                                }
                            />
                        </div>
                    )}
                </For>
                <Show when={props.onFork}>
                    <button
                        type="button"
                        class="fork-bar-add"
                        title="Fork this conversation"
                        aria-label="Fork this conversation"
                        onClick={() => props.onFork!()}
                    >
                        + fork
                    </button>
                </Show>
            </div>
        </Show>
    );
};

ForkBar.displayName = "ForkBar";
